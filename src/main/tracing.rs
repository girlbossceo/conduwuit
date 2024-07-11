use std::sync::Arc;

use conduit::{
	config,
	config::Config,
	debug_warn,
	log::{capture, LogLevelReloadHandles},
};
use tracing_subscriber::{layer::SubscriberExt, reload, EnvFilter, Layer, Registry};

#[cfg(feature = "perf_measurements")]
pub(crate) type TracingFlameGuard = Option<tracing_flame::FlushGuard<std::io::BufWriter<std::fs::File>>>;
#[cfg(not(feature = "perf_measurements"))]
pub(crate) type TracingFlameGuard = ();

#[allow(clippy::redundant_clone)]
pub(crate) fn init(config: &Config) -> (LogLevelReloadHandles, TracingFlameGuard, Arc<capture::State>) {
	let reload_handles = LogLevelReloadHandles::default();

	let fmt_layer = tracing_subscriber::fmt::Layer::new();
	let filter_layer = match EnvFilter::try_new(&config.log) {
		Ok(s) => s,
		Err(e) => {
			eprintln!("It looks like your config is invalid. The following error occured while parsing it: {e}");
			EnvFilter::try_new(config::default_log()).expect("failed to set default EnvFilter")
		},
	};

	let (fmt_reload_filter, fmt_reload_handle) = reload::Layer::new(filter_layer.clone());
	reload_handles.add("console", Box::new(fmt_reload_handle));

	let subscriber = Registry::default().with(fmt_layer.with_filter(fmt_reload_filter));

	let cap_state = Arc::new(capture::State::new());
	let cap_layer = capture::Layer::new(&cap_state);
	let subscriber = subscriber.with(cap_layer);

	#[cfg(feature = "sentry_telemetry")]
	let subscriber = {
		let sentry_layer = sentry_tracing::layer();
		let (sentry_reload_filter, sentry_reload_handle) = reload::Layer::new(filter_layer.clone());
		reload_handles.add("sentry", Box::new(sentry_reload_handle));
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
			reload_handles.add("jaeger", Box::new(jaeger_reload_handle));
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

	let ret = (reload_handles, flame_guard, cap_state);

	// Enable the tokio console. This is slightly kludgy because we're judggling
	// compile-time and runtime conditions to elide it, each of those changing the
	// subscriber's type.
	let (console_enabled, console_disabled_reason) = tokio_console_enabled(config);
	#[cfg(all(feature = "tokio_console", tokio_unstable))]
	if console_enabled {
		let console_layer = console_subscriber::ConsoleLayer::builder()
			.with_default_env()
			.spawn();

		set_global_default(subscriber.with(console_layer));
		return ret;
	}

	set_global_default(subscriber);

	// If there's a reason the tokio console was disabled when it might be desired
	// we output that here after initializing logging
	if !console_enabled && !console_disabled_reason.is_empty() {
		debug_warn!("{console_disabled_reason}");
	}

	ret
}

fn tokio_console_enabled(config: &Config) -> (bool, &'static str) {
	if !cfg!(all(feature = "tokio_console", tokio_unstable)) {
		return (false, "");
	}

	if cfg!(feature = "release_max_log_level") && !cfg!(debug_assertions) {
		return (
			false,
			"'tokio_console' feature and 'release_max_log_level' feature are incompatible.",
		);
	}

	if !config.tokio_console {
		return (false, "tokio console is available but disabled by the configuration.");
	}

	(true, "")
}

fn set_global_default<S: SubscriberExt + Send + Sync>(subscriber: S) {
	tracing::subscriber::set_global_default(subscriber)
		.expect("the global default tracing subscriber failed to be initialized");
}
