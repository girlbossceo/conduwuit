use std::sync::Arc;

use conduit::{
	config,
	config::Config,
	log::{capture, LogLevelReloadHandles, ReloadHandle},
};
use tracing_subscriber::{prelude::*, reload, EnvFilter, Registry};

#[cfg(feature = "perf_measurements")]
pub(crate) type TracingFlameGuard = Option<tracing_flame::FlushGuard<std::io::BufWriter<std::fs::File>>>;
#[cfg(not(feature = "perf_measurements"))]
pub(crate) type TracingFlameGuard = ();

#[allow(clippy::redundant_clone)]
pub(crate) fn init(config: &Config) -> (LogLevelReloadHandles, TracingFlameGuard, Arc<capture::State>) {
	let registry = Registry::default();
	let fmt_layer = tracing_subscriber::fmt::Layer::new();
	let filter_layer = match EnvFilter::try_new(&config.log) {
		Ok(s) => s,
		Err(e) => {
			eprintln!("It looks like your config is invalid. The following error occured while parsing it: {e}");
			EnvFilter::try_new(config::default_log()).expect("failed to set default EnvFilter")
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

	let cap_state = Arc::new(capture::State::new());
	let cap_layer = capture::Layer::new(&cap_state);
	let subscriber = subscriber
		.with(fmt_layer.with_filter(fmt_reload_filter))
		.with(cap_layer);

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
					Err(e) => panic!("failed to initialize tracing-flame: {e}"),
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

			let (jaeger_reload_filter, jaeger_reload_handle) = reload::Layer::new(filter_layer.clone());
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

	tracing::subscriber::set_global_default(subscriber).expect("failed to set global tracing subscriber");

	#[cfg(all(feature = "tokio_console", feature = "release_max_log_level", tokio_unstable))]
	tracing::error!(
		"'tokio_console' feature and 'release_max_log_level' feature are incompatible, because console-subscriber \
		 needs access to trace-level events. 'release_max_log_level' must be disabled to use tokio-console."
	);

	(LogLevelReloadHandles::new(reload_handles), flame_guard, cap_state)
}
