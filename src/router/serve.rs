use std::{
	net::SocketAddr,
	path::Path,
	sync::{atomic::Ordering, Arc},
};

use axum::{extract::Request, routing::IntoMakeService, Router};
use axum_server::{bind, bind_rustls, tls_rustls::RustlsConfig, Handle as ServerHandle};
#[cfg(feature = "axum_dual_protocol")]
use axum_server_dual_protocol::ServerExt;
use conduit::{debug_error, debug_info, utils, Error, Result, Server};
use hyper::{body::Incoming, service::service_fn};
use hyper_util::{
	rt::{TokioExecutor, TokioIo},
	server,
};
use tokio::{
	fs,
	sync::broadcast::{self},
	task::JoinSet,
};
use tower::{Service, ServiceExt};
use tracing::{debug, info, warn};
use utils::unwrap_infallible;

pub(crate) async fn plain(
	server: &Arc<Server>, app: IntoMakeService<Router>, handle: ServerHandle, addrs: Vec<SocketAddr>,
) -> Result<()> {
	let mut join_set = JoinSet::new();
	for addr in &addrs {
		join_set.spawn_on(bind(*addr).handle(handle.clone()).serve(app.clone()), server.runtime());
	}

	info!("Listening on {addrs:?}");
	while join_set.join_next().await.is_some() {}

	let spawn_active = server.requests_spawn_active.load(Ordering::Relaxed);
	let handle_active = server.requests_handle_active.load(Ordering::Relaxed);
	debug_info!(
		spawn_finished = server.requests_spawn_finished.load(Ordering::Relaxed),
		handle_finished = server.requests_handle_finished.load(Ordering::Relaxed),
		panics = server.requests_panic.load(Ordering::Relaxed),
		spawn_active,
		handle_active,
		"Stopped listening on {addrs:?}",
	);

	debug_assert!(spawn_active == 0, "active request tasks are not joined");
	debug_assert!(handle_active == 0, "active request handles still pending");

	Ok(())
}

pub(crate) async fn tls(
	server: &Arc<Server>, app: IntoMakeService<Router>, handle: ServerHandle, addrs: Vec<SocketAddr>,
) -> Result<()> {
	let config = &server.config;
	let tls = config.tls.as_ref().expect("TLS configuration");

	debug!(
		"Using direct TLS. Certificate path {} and certificate private key path {}",
		&tls.certs, &tls.key
	);
	info!(
		"Note: It is strongly recommended that you use a reverse proxy instead of running conduwuit directly with TLS."
	);
	let conf = RustlsConfig::from_pem_file(&tls.certs, &tls.key).await?;

	if cfg!(feature = "axum_dual_protocol") {
		info!(
			"conduwuit was built with axum_dual_protocol feature to listen on both HTTP and HTTPS. This will only \
			 take effect if `dual_protocol` is enabled in `[global.tls]`"
		);
	}

	let mut join_set = JoinSet::new();
	if cfg!(feature = "axum_dual_protocol") && tls.dual_protocol {
		#[cfg(feature = "axum_dual_protocol")]
		for addr in &addrs {
			join_set.spawn_on(
				axum_server_dual_protocol::bind_dual_protocol(*addr, conf.clone())
					.set_upgrade(false)
					.handle(handle.clone())
					.serve(app.clone()),
				server.runtime(),
			);
		}
	} else {
		for addr in &addrs {
			join_set.spawn_on(
				bind_rustls(*addr, conf.clone())
					.handle(handle.clone())
					.serve(app.clone()),
				server.runtime(),
			);
		}
	}

	if cfg!(feature = "axum_dual_protocol") && tls.dual_protocol {
		warn!(
			"Listening on {:?} with TLS certificate {} and supporting plain text (HTTP) connections too (insecure!)",
			addrs, &tls.certs
		);
	} else {
		info!("Listening on {:?} with TLS certificate {}", addrs, &tls.certs);
	}

	while join_set.join_next().await.is_some() {}

	Ok(())
}

#[cfg(unix)]
pub(crate) async fn unix_socket(
	server: &Arc<Server>, app: IntoMakeService<Router>, mut shutdown: broadcast::Receiver<()>,
) -> Result<()> {
	let mut tasks = JoinSet::<()>::new();
	let executor = TokioExecutor::new();
	let builder = server::conn::auto::Builder::new(executor);
	let listener = unix_socket_init(server).await?;
	loop {
		let app = app.clone();
		let builder = builder.clone();
		tokio::select! {
			_sig = shutdown.recv() => break,
			accept = listener.accept() => match accept {
				Ok(conn) => unix_socket_accept(server, &listener, &mut tasks, app, builder, conn).await,
				Err(err) => debug_error!(?listener, "accept error: {err}"),
			},
		}
	}

	drop(listener);
	tasks.shutdown().await;

	Ok(())
}

#[cfg(unix)]
async fn unix_socket_accept(
	server: &Arc<Server>, listener: &tokio::net::UnixListener, tasks: &mut JoinSet<()>,
	mut app: IntoMakeService<Router>, builder: server::conn::auto::Builder<TokioExecutor>,
	conn: (tokio::net::UnixStream, tokio::net::unix::SocketAddr),
) {
	let (socket, remote) = conn;
	let socket = TokioIo::new(socket);
	debug!(?listener, ?socket, ?remote, "accepted");

	let called = unwrap_infallible(app.call(()).await);
	let handler = service_fn(move |req: Request<Incoming>| called.clone().oneshot(req));

	let task = async move {
		builder
			.serve_connection(socket, handler)
			.await
			.map_err(|e| debug_error!(?remote, "connection error: {e}"))
			.expect("connection error");
	};

	_ = tasks.spawn_on(task, server.runtime());
	while tasks.try_join_next().is_some() {}
}

#[cfg(unix)]
async fn unix_socket_init(server: &Arc<Server>) -> Result<tokio::net::UnixListener> {
	use std::os::unix::fs::PermissionsExt;

	let config = &server.config;
	let path = config
		.unix_socket_path
		.as_ref()
		.expect("failed to extract configured unix socket path");

	if path.exists() {
		warn!("Removing existing UNIX socket {:#?} (unclean shutdown?)...", path.display());
		fs::remove_file(&path)
			.await
			.map_err(|e| warn!("Failed to remove existing UNIX socket: {e}"))
			.unwrap();
	}

	let dir = path.parent().unwrap_or_else(|| Path::new("/"));
	if let Err(e) = fs::create_dir_all(dir).await {
		return Err(Error::Err(format!("Failed to create {dir:?} for socket {path:?}: {e}")));
	}

	let listener = tokio::net::UnixListener::bind(path);
	if let Err(e) = listener {
		return Err(Error::Err(format!("Failed to bind listener {path:?}: {e}")));
	}

	let socket_perms = config.unix_socket_perms.to_string();
	let octal_perms = u32::from_str_radix(&socket_perms, 8).expect("failed to convert octal permissions");
	let perms = std::fs::Permissions::from_mode(octal_perms);
	if let Err(e) = fs::set_permissions(&path, perms).await {
		return Err(Error::Err(format!("Failed to set socket {path:?} permissions: {e}")));
	}

	info!("Listening at {:?}", path);

	Ok(listener.unwrap())
}
