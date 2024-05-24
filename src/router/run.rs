use std::{sync::Arc, time::Duration};

use axum_server::Handle as ServerHandle;
use tokio::{
	signal,
	sync::oneshot::{self, Sender},
};
use tracing::{debug, info, warn};

extern crate conduit_admin as admin;
extern crate conduit_core as conduit;
extern crate conduit_database as database;
extern crate conduit_service as service;

use std::sync::atomic::Ordering;

use conduit::{debug_info, trace, Error, Result, Server};
use database::KeyValueDatabase;
use service::{services, Services};

use crate::{layers, serve};

/// Main loop base
#[tracing::instrument(skip_all)]
#[allow(clippy::let_underscore_must_use)] // various of these are intended
pub(crate) async fn run(server: Arc<Server>) -> Result<(), Error> {
	let config = &server.config;
	let app = layers::build(&server)?;
	let addrs = config.get_bind_addrs();

	// Install the admin room callback here for now
	_ = services().admin.handle.lock().await.insert(admin::handle);

	// Setup shutdown/signal handling
	let handle = ServerHandle::new();
	_ = server
		.shutdown
		.lock()
		.expect("locked")
		.insert(handle.clone());

	server.interrupt.store(false, Ordering::Release);
	let (tx, rx) = oneshot::channel::<()>();
	let sigs = server.runtime().spawn(sighandle(server.clone(), tx));

	// Prepare to serve http clients
	let res;
	// Serve clients
	if cfg!(unix) && config.unix_socket_path.is_some() {
		res = serve::unix_socket(&server, app, rx).await;
	} else if config.tls.is_some() {
		res = serve::tls(&server, app, handle.clone(), addrs).await;
	} else {
		res = serve::plain(&server, app, handle.clone(), addrs).await;
	}

	// Join the signal handler before we leave.
	sigs.abort();
	_ = sigs.await;

	// Reset the axum handle instance; this should be reusable and might be
	// reload-survivable but better to be safe than sorry.
	_ = server.shutdown.lock().expect("locked").take();

	// Remove the admin room callback
	_ = services().admin.handle.lock().await.take();

	debug_info!("Finished");
	Ok(res?)
}

/// Async initializations
#[tracing::instrument(skip_all)]
#[allow(clippy::let_underscore_must_use)]
pub(crate) async fn start(server: Arc<Server>) -> Result<(), Error> {
	debug!("Starting...");
	let d = Arc::new(KeyValueDatabase::load_or_create(&server).await?);
	let s = Box::new(Services::build(server, d.clone()).await?);
	_ = service::SERVICES
		.write()
		.expect("write locked")
		.insert(Box::leak(s));
	services().start().await?;

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
	services().shutdown().await;

	// Deactivate services(). Any further use will panic the caller.
	let s = service::SERVICES
		.write()
		.expect("write locked")
		.take()
		.unwrap();

	let s = std::ptr::from_ref(s) as *mut Services;
	//SAFETY: Services was instantiated in start() and leaked into the SERVICES
	// global perusing as 'static for the duration of run_server(). Now we reclaim
	// it to drop it before unloading the module. If this is not done there will be
	// multiple instances after module reload.
	let s = unsafe { Box::from_raw(s) };
	debug!("Cleaning up...");
	// Drop it so we encounter any trouble before the infolog message
	drop(s);

	#[cfg(feature = "systemd")]
	sd_notify::notify(true, &[sd_notify::NotifyState::Stopping]).expect("failed to notify systemd of stopping state");

	info!("Shutdown complete.");
	Ok(())
}

#[tracing::instrument(skip_all)]
async fn sighandle(server: Arc<Server>, tx: Sender<()>) -> Result<(), Error> {
	let ctrl_c = async {
		signal::ctrl_c()
			.await
			.expect("failed to install Ctrl+C handler");

		let reload = cfg!(unix) && cfg!(debug_assertions);
		server.reload.store(reload, Ordering::Release);
	};

	#[cfg(unix)]
	let ctrl_bs = async {
		signal::unix::signal(signal::unix::SignalKind::quit())
			.expect("failed to install Ctrl+\\ handler")
			.recv()
			.await;
	};

	#[cfg(unix)]
	let terminate = async {
		signal::unix::signal(signal::unix::SignalKind::terminate())
			.expect("failed to install SIGTERM handler")
			.recv()
			.await;
	};

	debug!("Installed signal handlers");
	let sig: &str;
	#[cfg(unix)]
	tokio::select! {
		() = ctrl_c => { sig = "Ctrl+C"; },
		() = ctrl_bs => { sig = "Ctrl+\\"; },
		() = terminate => { sig = "SIGTERM"; },
	}

	#[cfg(not(unix))]
	tokio::select! {
		_ = ctrl_c => { sig = "Ctrl+C"; },
	}

	warn!("Received {}", sig);
	server.interrupt.store(true, Ordering::Release);
	services().globals.rotate.fire();
	tx.send(())
		.expect("failed sending shutdown transaction to oneshot channel");

	if let Some(handle) = server.shutdown.lock().expect("locked").as_ref() {
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

	Ok(())
}
