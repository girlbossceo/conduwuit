pub(crate) mod clap;
mod mods;
mod restart;
mod sentry;
mod server;
mod signal;
mod tracing;

extern crate conduit_core as conduit;

use std::{
	cmp,
	sync::{atomic::Ordering, Arc},
	time::Duration,
};

use conduit::{debug_info, error, rustc_flags_capture, utils::available_parallelism, Error, Result};
use server::Server;
use tokio::runtime;

const WORKER_NAME: &str = "conduwuit:worker";
const WORKER_MIN: usize = 2;
const WORKER_KEEPALIVE: u64 = 36;

rustc_flags_capture! {}

fn main() -> Result<(), Error> {
	let args = clap::parse();
	let runtime = runtime::Builder::new_multi_thread()
		.enable_io()
		.enable_time()
		.thread_name(WORKER_NAME)
		.worker_threads(cmp::max(WORKER_MIN, available_parallelism()))
		.thread_keep_alive(Duration::from_secs(WORKER_KEEPALIVE))
		.build()
		.expect("built runtime");

	let server: Arc<Server> = Server::build(&args, Some(runtime.handle()))?;
	runtime.spawn(signal::signal(server.clone()));
	runtime.block_on(async_main(&server))?;

	// explicit drop here to trace thread and tls dtors
	drop(runtime);

	#[cfg(unix)]
	if server.server.restarting.load(Ordering::Acquire) {
		restart::restart();
	}

	debug_info!("Exit");
	Ok(())
}

/// Operate the server normally in release-mode static builds. This will start,
/// run and stop the server within the asynchronous runtime.
#[cfg(not(conduit_mods))]
async fn async_main(server: &Arc<Server>) -> Result<(), Error> {
	extern crate conduit_router as router;

	match router::start(&server.server).await {
		Ok(services) => server.services.lock().await.insert(services),
		Err(error) => {
			error!("Critical error starting server: {error}");
			return Err(error);
		},
	};

	if let Err(error) = router::run(
		server
			.services
			.lock()
			.await
			.as_ref()
			.expect("services initialized"),
	)
	.await
	{
		error!("Critical error running server: {error}");
		return Err(error);
	}

	if let Err(error) = router::stop(
		server
			.services
			.lock()
			.await
			.take()
			.expect("services initialied"),
	)
	.await
	{
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
async fn async_main(server: &Arc<Server>) -> Result<(), Error> {
	let mut starts = true;
	let mut reloads = true;
	while reloads {
		if let Err(error) = mods::open(server).await {
			error!("Loading router: {error}");
			return Err(error);
		}

		let result = mods::run(server, starts).await;
		if let Ok(result) = result {
			(starts, reloads) = result;
		}

		let force = !reloads || result.is_err();
		if let Err(error) = mods::close(server, force).await {
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
