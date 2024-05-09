#[cfg(unix)]
use std::fs::Permissions; // only for UNIX sockets stuff and *nix container checks
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
use std::{
	io,
	net::SocketAddr,
	sync::{atomic::Ordering, Arc},
};

use axum::Router;
use axum_server::{bind, bind_rustls, tls_rustls::RustlsConfig, Handle as ServerHandle};
#[cfg(feature = "axum_dual_protocol")]
use axum_server_dual_protocol::ServerExt;
use conduit::{debug_info, Server};
use tokio::{
	sync::oneshot::{self},
	task::JoinSet,
};
use tracing::{debug, info, warn};

pub(crate) async fn plain(
	server: &Arc<Server>, app: axum::routing::IntoMakeService<Router>, handle: ServerHandle, addrs: Vec<SocketAddr>,
) -> io::Result<()> {
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
	server: &Arc<Server>, app: axum::routing::IntoMakeService<Router>, handle: ServerHandle, addrs: Vec<SocketAddr>,
) -> io::Result<()> {
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
#[allow(unused_variables)]
pub(crate) async fn unix_socket(
	server: &Arc<Server>, app: axum::routing::IntoMakeService<Router>, rx: oneshot::Receiver<()>,
) -> io::Result<()> {
	let config = &server.config;
	let path = config.unix_socket_path.as_ref().unwrap();

	if path.exists() {
		warn!(
			"UNIX socket path {:#?} already exists (unclean shutdown?), attempting to remove it.",
			path.display()
		);
		tokio::fs::remove_file(&path).await?;
	}

	tokio::fs::create_dir_all(path.parent().unwrap()).await?;

	let socket_perms = config.unix_socket_perms.to_string();
	let octal_perms = u32::from_str_radix(&socket_perms, 8).unwrap();
	tokio::fs::set_permissions(&path, Permissions::from_mode(octal_perms))
		.await
		.unwrap();

	let bind = tokio::net::UnixListener::bind(path)?;
	info!("Listening at {:?}", path);

	Ok(())
}
