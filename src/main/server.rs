use std::sync::Arc;

use conduwuit::{
	config::Config,
	info,
	log::Log,
	utils::{stream, sys},
	Error, Result,
};
use tokio::{runtime, sync::Mutex};

use crate::{clap::Args, logging::TracingFlameGuard};

/// Server runtime state; complete
pub(crate) struct Server {
	/// Server runtime state; public portion
	pub(crate) server: Arc<conduwuit::Server>,

	pub(crate) services: Mutex<Option<Arc<conduwuit_service::Services>>>,

	_tracing_flame_guard: TracingFlameGuard,

	#[cfg(feature = "sentry_telemetry")]
	_sentry_guard: Option<::sentry::ClientInitGuard>,

	#[cfg(conduwuit_mods)]
	// Module instances; TODO: move to mods::loaded mgmt vector
	pub(crate) mods: tokio::sync::RwLock<Vec<conduwuit::mods::Module>>,
}

impl Server {
	pub(crate) fn new(
		args: &Args,
		runtime: Option<&runtime::Handle>,
	) -> Result<Arc<Self>, Error> {
		let _runtime_guard = runtime.map(runtime::Handle::enter);

		let raw_config = Config::load(args.config.as_deref())?;
		let raw_config = crate::clap::update(raw_config, args)?;
		let config = Config::new(&raw_config)?;

		#[cfg(feature = "sentry_telemetry")]
		let sentry_guard = crate::sentry::init(&config);

		let (tracing_reload_handle, tracing_flame_guard, capture) =
			crate::logging::init(&config)?;

		config.check()?;

		#[cfg(unix)]
		sys::maximize_fd_limit()
			.expect("Unable to increase maximum soft and hard file descriptor limit");

		let (_old_width, _new_width) = stream::set_width(config.stream_width_default);

		info!(
			server_name = %config.server_name,
			database_path = ?config.database_path,
			log_levels = %config.log,
			"{}",
			conduwuit::version(),
		);

		Ok(Arc::new(Self {
			server: Arc::new(conduwuit::Server::new(config, runtime.cloned(), Log {
				reload: tracing_reload_handle,
				capture,
			})),

			services: None.into(),

			_tracing_flame_guard: tracing_flame_guard,

			#[cfg(feature = "sentry_telemetry")]
			_sentry_guard: sentry_guard,

			#[cfg(conduwuit_mods)]
			mods: tokio::sync::RwLock::new(Vec::new()),
		}))
	}
}
