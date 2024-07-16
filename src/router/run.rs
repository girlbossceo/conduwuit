use std::{sync::Arc, time::Duration};

use axum_server::Handle as ServerHandle;
use tokio::{
	sync::broadcast::{self, Sender},
	task::JoinHandle,
};

extern crate conduit_admin as admin;
extern crate conduit_core as conduit;
extern crate conduit_service as service;

use std::sync::atomic::Ordering;

use conduit::{debug, debug_info, error, info, Error, Result, Server};

use crate::serve;

/// Main loop base
#[tracing::instrument(skip_all)]
pub(crate) async fn run(server: Arc<Server>) -> Result<()> {
	debug!("Start");

	// Install the admin room callback here for now
	admin::init().await;

	// Setup shutdown/signal handling
	let handle = ServerHandle::new();
	let (tx, _) = broadcast::channel::<()>(1);
	let sigs = server
		.runtime()
		.spawn(signal(server.clone(), tx.clone(), handle.clone()));

	let mut listener = server
		.runtime()
		.spawn(serve::serve(server.clone(), handle.clone(), tx.subscribe()));

	// Focal point
	debug!("Running");
	let res = tokio::select! {
		res = &mut listener => res.map_err(Error::from).unwrap_or_else(Err),
		res = service::services().poll() => handle_services_poll(&server, res, listener).await,
	};

	// Join the signal handler before we leave.
	sigs.abort();
	_ = sigs.await;

	// Remove the admin room callback
	admin::fini().await;

	debug_info!("Finish");
	res
}

/// Async initializations
#[tracing::instrument(skip_all)]
pub(crate) async fn start(server: Arc<Server>) -> Result<()> {
	debug!("Starting...");

	service::start(&server).await?;

	#[cfg(feature = "systemd")]
	sd_notify::notify(true, &[sd_notify::NotifyState::Ready]).expect("failed to notify systemd of ready state");

	debug!("Started");
	Ok(())
}

/// Async destructions
#[tracing::instrument(skip_all)]
pub(crate) async fn stop(_server: Arc<Server>) -> Result<()> {
	debug!("Shutting down...");

	// Wait for all completions before dropping or we'll lose them to the module
	// unload and explode.
	service::stop().await;

	debug!("Cleaning up...");

	#[cfg(feature = "systemd")]
	sd_notify::notify(true, &[sd_notify::NotifyState::Stopping]).expect("failed to notify systemd of stopping state");

	info!("Shutdown complete.");
	Ok(())
}

#[tracing::instrument(skip_all)]
async fn signal(server: Arc<Server>, tx: Sender<()>, handle: axum_server::Handle) {
	loop {
		let sig: &'static str = server
			.signal
			.subscribe()
			.recv()
			.await
			.expect("channel error");

		if !server.running() {
			handle_shutdown(&server, &tx, &handle, sig).await;
			break;
		}
	}
}

async fn handle_shutdown(server: &Arc<Server>, tx: &Sender<()>, handle: &axum_server::Handle, sig: &str) {
	debug!("Received signal {sig}");
	if let Err(e) = tx.send(()) {
		error!("failed sending shutdown transaction to channel: {e}");
	}

	let timeout = Duration::from_secs(36);
	debug!(
		?timeout,
		spawn_active = ?server.metrics.requests_spawn_active.load(Ordering::Relaxed),
		handle_active = ?server.metrics.requests_handle_active.load(Ordering::Relaxed),
		"Notifying for graceful shutdown"
	);

	handle.graceful_shutdown(Some(timeout));
}

async fn handle_services_poll(
	server: &Arc<Server>, result: Result<()>, listener: JoinHandle<Result<()>>,
) -> Result<()> {
	debug!("Service manager finished: {result:?}");

	if server.running() {
		if let Err(e) = server.shutdown() {
			error!("Failed to send shutdown signal: {e}");
		}
	}

	if let Err(e) = listener.await {
		error!("Client listener task finished with error: {e}");
	}

	result
}
