mod mods;
mod server;

extern crate conduit_core as conduit;

use std::{cmp, sync::Arc, time::Duration};

use conduit::{debug_info, error, utils::clap, Error, Result};
use server::Server;
use tokio::runtime;

const WORKER_NAME: &str = "conduwuit:worker";
const WORKER_MIN: usize = 2;
const WORKER_KEEPALIVE_MS: u64 = 2500;

fn main() -> Result<(), Error> {
	let args = clap::parse();
	let runtime = runtime::Builder::new_multi_thread()
		.enable_io()
		.enable_time()
		.thread_name(WORKER_NAME)
		.worker_threads(cmp::max(WORKER_MIN, num_cpus::get()))
		.thread_keep_alive(Duration::from_millis(WORKER_KEEPALIVE_MS))
		.build()
		.expect("built runtime");

	let handle = runtime.handle();
	let server: Arc<Server> = Server::build(args, Some(handle))?;
	runtime.block_on(async { async_main(server.clone()).await })?;

	// explicit drop here to trace thread and tls dtors
	drop(runtime);

	debug_info!("Exit");
	Ok(())
}

/// Operate the server normally in release-mode static builds. This will start,
/// run and stop the server within the asynchronous runtime.
#[cfg(not(conduit_mods))]
async fn async_main(server: Arc<Server>) -> Result<(), Error> {
	extern crate conduit_router as router;
	use tracing::error;

	if let Err(error) = router::start(&server.server).await {
		error!("Critical error starting server: {error}");
		return Err(error);
	}

	if let Err(error) = router::run(&server.server).await {
		error!("Critical error running server: {error}");
		return Err(error);
	}

	if let Err(error) = router::stop(&server.server).await {
		error!("Critical error stopping server: {error}");
		return Err(error);
	}

	debug_info!("Exit runtime");
	Ok(())
}

/// Operate the server in developer-mode dynamic builds. This will start, run,
/// and hot-reload portions of the server as-needed before returning for an
/// actual shutdown. This is not available in release-mode or static builds.
#[cfg(conduit_mods)]
async fn async_main(server: Arc<Server>) -> Result<(), Error> {
	let mut starts = true;
	let mut reloads = true;
	while reloads {
		if let Err(error) = mods::open(&server).await {
			error!("Loading router: {error}");
			return Err(error);
		}

		let result = mods::run(&server, starts).await;
		if let Ok(result) = result {
			(starts, reloads) = result;
		}

		let force = !reloads || result.is_err();
		if let Err(error) = mods::close(&server, force).await {
			error!("Unloading router: {error}");
			return Err(error);
		}

		if let Err(error) = result {
			error!("{error}");
			return Err(error);
		}
	}

	debug_info!("Exit runtime");
	Ok(())
}
