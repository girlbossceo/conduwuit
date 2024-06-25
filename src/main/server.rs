use std::sync::Arc;

use conduit::{
	config::Config,
	info,
	log::{self},
	utils::sys,
	Error, Result,
};
use tokio::runtime;

use crate::{clap::Args, tracing::TracingFlameGuard};

/// Server runtime state; complete
pub(crate) struct Server {
	/// Server runtime state; public portion
	pub(crate) server: Arc<conduit::Server>,

	_tracing_flame_guard: TracingFlameGuard,

	#[cfg(feature = "sentry_telemetry")]
	_sentry_guard: Option<::sentry::ClientInitGuard>,

	#[cfg(conduit_mods)]
	// Module instances; TODO: move to mods::loaded mgmt vector
	pub(crate) mods: tokio::sync::RwLock<Vec<conduit::mods::Module>>,
}

impl Server {
	pub(crate) fn build(args: Args, runtime: Option<&runtime::Handle>) -> Result<Arc<Self>, Error> {
		let config = Config::new(args.config)?;

		#[cfg(feature = "sentry_telemetry")]
		let sentry_guard = crate::sentry::init(&config);

		let (tracing_reload_handle, tracing_flame_guard, capture) = crate::tracing::init(&config);

		config.check()?;

		#[cfg(unix)]
		sys::maximize_fd_limit().expect("Unable to increase maximum soft and hard file descriptor limit");

		info!(
			server_name = %config.server_name,
			database_path = ?config.database_path,
			log_levels = %config.log,
			"{}",
			conduit::version(),
		);

		Ok(Arc::new(Self {
			server: Arc::new(conduit::Server::new(
				config,
				runtime.cloned(),
				log::Server {
					reload: tracing_reload_handle,
					capture,
				},
			)),

			_tracing_flame_guard: tracing_flame_guard,

			#[cfg(feature = "sentry_telemetry")]
			_sentry_guard: sentry_guard,

			#[cfg(conduit_mods)]
			mods: tokio::sync::RwLock::new(Vec::new()),
		}))
	}
}
