use std::{
	net::SocketAddr,
	sync::{atomic::Ordering, Arc},
};

use axum::{routing::IntoMakeService, Router};
use axum_server::{bind, Handle as ServerHandle};
use conduit::{debug_info, Result, Server};
use tokio::task::JoinSet;
use tracing::info;

pub(super) async fn serve(
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
