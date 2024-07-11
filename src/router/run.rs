use std::{sync::Arc, time::Duration};

use axum_server::Handle as ServerHandle;
use tokio::sync::broadcast::{self, Sender};
use tracing::{debug, error, info};

extern crate conduit_admin as admin;
extern crate conduit_core as conduit;
extern crate conduit_service as service;

use std::sync::atomic::Ordering;

use conduit::{debug_info, trace, Error, Result, Server};

use crate::{layers, serve};

/// Main loop base
#[tracing::instrument(skip_all)]
pub(crate) async fn run(server: Arc<Server>) -> Result<(), Error> {
	let app = layers::build(&server)?;

	// Install the admin room callback here for now
	admin::init().await;

	// Setup shutdown/signal handling
	let handle = ServerHandle::new();
	let (tx, _) = broadcast::channel::<()>(1);
	let sigs = server
		.runtime()
		.spawn(signal(server.clone(), tx.clone(), handle.clone()));

	// Serve clients
	let res = serve::serve(&server, app, handle, tx.subscribe()).await;

	// Join the signal handler before we leave.
	sigs.abort();
	_ = sigs.await;

	// Remove the admin room callback
	admin::fini().await;

	debug_info!("Finished");
	res
}

/// Async initializations
#[tracing::instrument(skip_all)]
pub(crate) async fn start(server: Arc<Server>) -> Result<(), Error> {
	debug!("Starting...");

	service::init(&server).await?;

	#[cfg(feature = "systemd")]
	sd_notify::notify(true, &[sd_notify::NotifyState::Ready]).expect("failed to notify systemd of ready state");

	debug!("Started");
	Ok(())
}

/// Async destructions
#[tracing::instrument(skip_all)]
pub(crate) async fn stop(_server: Arc<Server>) -> Result<(), Error> {
	debug!("Shutting down...");

	// Wait for all completions before dropping or we'll lose them to the module
	// unload and explode.
	service::fini().await;

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
	debug!("Received signal {}", sig);
	if let Err(e) = tx.send(()) {
		error!("failed sending shutdown transaction to channel: {e}");
	}

	let pending = server.requests_spawn_active.load(Ordering::Relaxed);
	if pending > 0 {
		let timeout = Duration::from_secs(36);
		trace!(pending, ?timeout, "Notifying for graceful shutdown");
		handle.graceful_shutdown(Some(timeout));
	} else {
		debug!(pending, "Notifying for immediate shutdown");
		handle.shutdown();
	}
}
