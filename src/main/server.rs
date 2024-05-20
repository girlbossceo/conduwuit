use std::sync::Arc;

use conduit::{
	conduwuit_version,
	config::Config,
	info,
	log::{LogLevelReloadHandles, ReloadHandle},
	utils::maximize_fd_limit,
	Error, Result,
};
use tokio::runtime;
use tracing_subscriber::{prelude::*, reload, EnvFilter, Registry};

use crate::clap::Args;

/// Server runtime state; complete
pub(crate) struct Server {
	/// Server runtime state; public portion
	pub(crate) server: Arc<conduit::Server>,

	_tracing_flame_guard: TracingFlameGuard,

	#[cfg(feature = "sentry_telemetry")]
	_sentry_guard: Option<sentry::ClientInitGuard>,

	#[cfg(conduit_mods)]
	// Module instances; TODO: move to mods::loaded mgmt vector
	pub(crate) mods: tokio::sync::RwLock<Vec<conduit::mods::Module>>,
}

impl Server {
	pub(crate) fn build(args: Args, runtime: Option<&runtime::Handle>) -> Result<Arc<Server>, Error> {
		let config = Config::new(args.config)?;
		#[cfg(feature = "sentry_telemetry")]
		let sentry_guard = init_sentry(&config);
		let (tracing_reload_handle, tracing_flame_guard) = init_tracing(&config);

		config.check()?;
		#[cfg(unix)]
		maximize_fd_limit().expect("Unable to increase maximum soft and hard file descriptor limit");
		info!(
			server_name = %config.server_name,
			database_path = ?config.database_path,
			log_levels = %config.log,
			"{}",
			conduwuit_version(),
		);

		Ok(Arc::new(Server {
			server: Arc::new(conduit::Server::new(config, runtime.cloned(), tracing_reload_handle)),

			_tracing_flame_guard: tracing_flame_guard,

			#[cfg(feature = "sentry_telemetry")]
			_sentry_guard: sentry_guard,

			#[cfg(conduit_mods)]
			mods: tokio::sync::RwLock::new(Vec::new()),
		}))
	}
}

#[cfg(feature = "sentry_telemetry")]
fn init_sentry(config: &Config) -> Option<sentry::ClientInitGuard> {
	if !config.sentry {
		return None;
	}

	let sentry_endpoint = config
		.sentry_endpoint
		.as_ref()
		.expect("init_sentry should only be called if sentry is enabled and this is not None")
		.as_str();

	let server_name = if config.sentry_send_server_name {
		Some(config.server_name.to_string().into())
	} else {
		None
	};

	Some(sentry::init((
		sentry_endpoint,
		sentry::ClientOptions {
			release: sentry::release_name!(),
			traces_sample_rate: config.sentry_traces_sample_rate,
			server_name,
			..Default::default()
		},
	)))
}

#[cfg(feature = "perf_measurements")]
type TracingFlameGuard = Option<tracing_flame::FlushGuard<std::io::BufWriter<std::fs::File>>>;
#[cfg(not(feature = "perf_measurements"))]
type TracingFlameGuard = ();

// clippy thinks the filter_layer clones are redundant if the next usage is
// behind a disabled feature.
#[allow(clippy::redundant_clone)]
fn init_tracing(config: &Config) -> (LogLevelReloadHandles, TracingFlameGuard) {
	let registry = Registry::default();
	let fmt_layer = tracing_subscriber::fmt::Layer::new();
	let filter_layer = match EnvFilter::try_new(&config.log) {
		Ok(s) => s,
		Err(e) => {
			eprintln!("It looks like your config is invalid. The following error occured while parsing it: {e}");
			EnvFilter::try_new("warn").unwrap()
		},
	};

	let mut reload_handles = Vec::<Box<dyn ReloadHandle<EnvFilter> + Send + Sync>>::new();
	let subscriber = registry;

	#[cfg(all(feature = "tokio_console", tokio_unstable))]
	let subscriber = {
		let console_layer = console_subscriber::spawn();
		subscriber.with(console_layer)
	};

	let (fmt_reload_filter, fmt_reload_handle) = reload::Layer::new(filter_layer.clone());
	reload_handles.push(Box::new(fmt_reload_handle));
	let subscriber = subscriber.with(fmt_layer.with_filter(fmt_reload_filter));

	#[cfg(feature = "sentry_telemetry")]
	let subscriber = {
		let sentry_layer = sentry_tracing::layer();
		let (sentry_reload_filter, sentry_reload_handle) = reload::Layer::new(filter_layer.clone());
		reload_handles.push(Box::new(sentry_reload_handle));
		subscriber.with(sentry_layer.with_filter(sentry_reload_filter))
	};

	#[cfg(feature = "perf_measurements")]
	let (subscriber, flame_guard) = {
		let (flame_layer, flame_guard) = if config.tracing_flame {
			let flame_filter = match EnvFilter::try_new(&config.tracing_flame_filter) {
				Ok(flame_filter) => flame_filter,
				Err(e) => panic!("tracing_flame_filter config value is invalid: {e}"),
			};

			let (flame_layer, flame_guard) =
				match tracing_flame::FlameLayer::with_file(&config.tracing_flame_output_path) {
					Ok(ok) => ok,
					Err(e) => {
						panic!("failed to initialize tracing-flame: {e}");
					},
				};
			let flame_layer = flame_layer
				.with_empty_samples(false)
				.with_filter(flame_filter);
			(Some(flame_layer), Some(flame_guard))
		} else {
			(None, None)
		};

		let jaeger_layer = if config.allow_jaeger {
			opentelemetry::global::set_text_map_propagator(opentelemetry_jaeger::Propagator::new());
			let tracer = opentelemetry_jaeger::new_agent_pipeline()
				.with_auto_split_batch(true)
				.with_service_name("conduwuit")
				.install_batch(opentelemetry_sdk::runtime::Tokio)
				.unwrap();
			let telemetry = tracing_opentelemetry::layer().with_tracer(tracer);

			let (jaeger_reload_filter, jaeger_reload_handle) = reload::Layer::new(filter_layer);
			reload_handles.push(Box::new(jaeger_reload_handle));
			Some(telemetry.with_filter(jaeger_reload_filter))
		} else {
			None
		};

		let subscriber = subscriber.with(flame_layer).with(jaeger_layer);
		(subscriber, flame_guard)
	};

	#[cfg(not(feature = "perf_measurements"))]
	#[cfg_attr(not(feature = "perf_measurements"), allow(clippy::let_unit_value))]
	let flame_guard = ();

	tracing::subscriber::set_global_default(subscriber).unwrap();

	#[cfg(all(feature = "tokio_console", feature = "release_max_log_level", tokio_unstable))]
	tracing::error!(
		"'tokio_console' feature and 'release_max_log_level' feature are incompatible, because console-subscriber \
		 needs access to trace-level events. 'release_max_log_level' must be disabled to use tokio-console."
	);

	(LogLevelReloadHandles::new(reload_handles), flame_guard)
}
