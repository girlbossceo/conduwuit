#![cfg(unix)]

use std::{path::Path, sync::Arc};

use axum::{extract::Request, routing::IntoMakeService, Router};
use conduit::{debug_error, utils, Error, Result, Server};
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

pub(super) async fn serve(
	server: &Arc<Server>, app: IntoMakeService<Router>, mut shutdown: broadcast::Receiver<()>,
) -> Result<()> {
	let mut tasks = JoinSet::<()>::new();
	let executor = TokioExecutor::new();
	let builder = server::conn::auto::Builder::new(executor);
	let listener = init(server).await?;
	loop {
		let app = app.clone();
		let builder = builder.clone();
		tokio::select! {
			_sig = shutdown.recv() => break,
			conn = listener.accept() => match conn {
				Ok(conn) => accept(server, &listener, &mut tasks, app, builder, conn).await,
				Err(err) => debug_error!(?listener, "accept error: {err}"),
			},
		}
	}

	drop(listener);
	tasks.shutdown().await;

	Ok(())
}

async fn accept(
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

async fn init(server: &Arc<Server>) -> Result<tokio::net::UnixListener> {
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
