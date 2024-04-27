#[cfg(unix)]
use std::fs::Permissions; // not unix specific, just only for UNIX sockets stuff and *nix container checks
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _; /* not unix specific, just only for UNIX sockets stuff and *nix
                                             * container checks */
// Not async due to services() being used in many closures, and async closures
// are not stable as of writing This is the case for every other occurence of
// sync Mutex/RwLock, except for database related ones
use std::sync::{Arc, RwLock};
use std::{any::Any, fs, io, net::SocketAddr, sync::atomic, time::Duration};

use api::ruma_wrapper::{Ruma, RumaResponse};
use axum::{
	extract::{DefaultBodyLimit, MatchedPath},
	response::IntoResponse,
	Router,
};
use axum_server::{bind, bind_rustls, tls_rustls::RustlsConfig, Handle as ServerHandle};
#[cfg(feature = "axum_dual_protocol")]
use axum_server_dual_protocol::ServerExt;
use config::Config;
use database::KeyValueDatabase;
use http::{
	header::{self, HeaderName},
	Method, StatusCode, Uri,
};
#[cfg(unix)]
use ruma::api::client::{
	error::{Error as RumaError, ErrorBody, ErrorKind},
	uiaa::UiaaResponse,
};
use service::{pdu::PduEvent, Services};
use tokio::{
	signal,
	sync::oneshot::{self, Sender},
	task::JoinSet,
};
use tower::ServiceBuilder;
use tower_http::{
	catch_panic::CatchPanicLayer,
	cors::{self, CorsLayer},
	trace::{DefaultOnFailure, DefaultOnRequest, DefaultOnResponse, TraceLayer},
	ServiceBuilderExt as _,
};
use tracing::{debug, error, info, trace, warn, Level};
use tracing_subscriber::{prelude::*, reload, EnvFilter, Registry};
use utils::{
	clap,
	error::{Error, Result},
};

mod api;
mod config;
mod database;
mod routes;
mod service;
mod utils;

pub(crate) static SERVICES: RwLock<Option<&'static Services<'static>>> = RwLock::new(None);

#[must_use]
pub(crate) fn services() -> &'static Services<'static> {
	SERVICES
		.read()
		.unwrap()
		.expect("SERVICES should be initialized when this is called")
}

#[cfg(all(not(target_env = "msvc"), feature = "jemalloc", not(feature = "hardened_malloc")))]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

#[cfg(all(not(target_env = "msvc"), feature = "hardened_malloc", target_os = "linux", not(feature = "jemalloc")))]
#[global_allocator]
static GLOBAL: hardened_malloc_rs::HardenedMalloc = hardened_malloc_rs::HardenedMalloc;

struct Server {
	config: Config,

	runtime: tokio::runtime::Runtime,

	tracing_reload_handle: LogLevelReloadHandles,

	#[cfg(feature = "sentry_telemetry")]
	_sentry_guard: Option<sentry::ClientInitGuard>,

	_tracing_flame_guard: TracingFlameGuard,
}

fn main() -> Result<(), Error> {
	let args = clap::parse();
	let conduwuit: Server = init(args)?;

	conduwuit
		.runtime
		.block_on(async { async_main(&conduwuit).await })
}

async fn async_main(server: &Server) -> Result<(), Error> {
	if let Err(error) = start(server).await {
		error!("Critical error starting server: {error}");
		return Err(Error::Err(format!("{error}")));
	}

	if let Err(error) = run(server).await {
		error!("Critical error running server: {error}");
		return Err(Error::Err(format!("{error}")));
	}

	if let Err(error) = stop(server).await {
		error!("Critical error stopping server: {error}");
		return Err(Error::Err(format!("{error}")));
	}

	Ok(())
}

async fn run(server: &Server) -> io::Result<()> {
	let app = build(server).await?;
	let (tx, rx) = oneshot::channel::<()>();
	let handle = ServerHandle::new();
	tokio::spawn(shutdown(handle.clone(), tx));

	#[cfg(unix)]
	if server.config.unix_socket_path.is_some() {
		return run_unix_socket_server(server, app, rx).await;
	}

	let addrs = server.config.get_bind_addrs();
	if server.config.tls.is_some() {
		return run_tls_server(server, app, handle, addrs).await;
	}

	let mut join_set = JoinSet::new();
	for addr in &addrs {
		join_set.spawn(bind(*addr).handle(handle.clone()).serve(app.clone()));
	}

	#[allow(clippy::let_underscore_untyped)] // error[E0658]: attributes on expressions are experimental
	#[cfg(feature = "systemd")]
	let _ = sd_notify::notify(true, &[sd_notify::NotifyState::Ready]);

	info!("Listening on {:?}", addrs);
	join_set.join_next().await;

	Ok(())
}

async fn run_tls_server(
	server: &Server, app: axum::routing::IntoMakeService<Router>, handle: ServerHandle, addrs: Vec<SocketAddr>,
) -> io::Result<()> {
	let tls = server.config.tls.as_ref().unwrap();

	debug!(
		"Using direct TLS. Certificate path {} and certificate private key path {}",
		&tls.certs, &tls.key
	);
	info!(
		"Note: It is strongly recommended that you use a reverse proxy instead of running conduwuit directly with TLS."
	);
	let conf = RustlsConfig::from_pem_file(&tls.certs, &tls.key).await?;

	if cfg!(feature = "axum_dual_protocol") {
		info!(
			"conduwuit was built with axum_dual_protocol feature to listen on both HTTP and HTTPS. This will only \
			 take affect if `dual_protocol` is enabled in `[global.tls]`"
		);
	}

	let mut join_set = JoinSet::new();

	if cfg!(feature = "axum_dual_protocol") && tls.dual_protocol {
		#[cfg(feature = "axum_dual_protocol")]
		for addr in &addrs {
			join_set.spawn(
				axum_server_dual_protocol::bind_dual_protocol(*addr, conf.clone())
					.set_upgrade(false)
					.handle(handle.clone())
					.serve(app.clone()),
			);
		}
	} else {
		for addr in &addrs {
			join_set.spawn(
				bind_rustls(*addr, conf.clone())
					.handle(handle.clone())
					.serve(app.clone()),
			);
		}
	}

	#[allow(clippy::let_underscore_untyped)] // error[E0658]: attributes on expressions are experimental
	#[cfg(feature = "systemd")]
	let _ = sd_notify::notify(true, &[sd_notify::NotifyState::Ready]);

	if cfg!(feature = "axum_dual_protocol") && tls.dual_protocol {
		warn!(
			"Listening on {:?} with TLS certificate {} and supporting plain text (HTTP) connections too (insecure!)",
			addrs, &tls.certs
		);
	} else {
		info!("Listening on {:?} with TLS certificate {}", addrs, &tls.certs);
	}

	join_set.join_next().await;

	Ok(())
}

#[cfg(unix)]
#[allow(unused_variables)]
async fn run_unix_socket_server(
	server: &Server, app: axum::routing::IntoMakeService<Router>, rx: oneshot::Receiver<()>,
) -> io::Result<()> {
	let path = server.config.unix_socket_path.as_ref().unwrap();

	if path.exists() {
		warn!(
			"UNIX socket path {:#?} already exists (unclean shutdown?), attempting to remove it.",
			path.display()
		);
		tokio::fs::remove_file(&path).await?;
	}

	tokio::fs::create_dir_all(path.parent().unwrap()).await?;

	let socket_perms = server.config.unix_socket_perms.to_string();
	let octal_perms = u32::from_str_radix(&socket_perms, 8).unwrap();
	tokio::fs::set_permissions(&path, Permissions::from_mode(octal_perms))
		.await
		.unwrap();

	#[allow(clippy::let_underscore_untyped)] // error[E0658]: attributes on expressions are experimental
	#[cfg(feature = "systemd")]
	let _ = sd_notify::notify(true, &[sd_notify::NotifyState::Ready]);
	let bind = tokio::net::UnixListener::bind(path)?;
	info!("Listening at {:?}", path);

	Ok(())
}

async fn shutdown(handle: ServerHandle, tx: Sender<()>) -> Result<()> {
	let ctrl_c = async {
		signal::ctrl_c()
			.await
			.expect("failed to install Ctrl+C handler");
	};

	#[cfg(unix)]
	let terminate = async {
		signal::unix::signal(signal::unix::SignalKind::terminate())
			.expect("failed to install SIGTERM handler")
			.recv()
			.await;
	};

	let sig: &str;
	#[cfg(unix)]
	tokio::select! {
		() = ctrl_c => { sig = "Ctrl+C"; },
		() = terminate => { sig = "SIGTERM"; },
	}
	#[cfg(not(unix))]
	tokio::select! {
		_ = ctrl_c => { sig = "Ctrl+C"; },
	}

	warn!("Received {}, shutting down...", sig);
	handle.graceful_shutdown(Some(Duration::from_secs(180)));
	services().globals.shutdown();

	#[allow(clippy::let_underscore_untyped)] // error[E0658]: attributes on expressions are experimental
	#[cfg(feature = "systemd")]
	let _ = sd_notify::notify(true, &[sd_notify::NotifyState::Stopping]);

	tx.send(()).expect(
		"failed sending shutdown transaction to oneshot channel (this is unlikely a conduwuit bug and more so your \
		 system may not be in an okay/ideal state.)",
	);

	Ok(())
}

async fn stop(_server: &Server) -> io::Result<()> {
	info!("Shutdown complete.");

	Ok(())
}

/// Async initializations
async fn start(server: &Server) -> Result<(), Error> {
	KeyValueDatabase::load_or_create(server.config.clone(), server.tracing_reload_handle.clone()).await?;

	Ok(())
}

async fn build(server: &Server) -> io::Result<axum::routing::IntoMakeService<Router>> {
	let base_middlewares = ServiceBuilder::new();
	#[cfg(feature = "sentry_telemetry")]
	let base_middlewares = base_middlewares.layer(sentry_tower::NewSentryLayer::<http::Request<_>>::new_from_top());

	let x_forwarded_for = HeaderName::from_static("x-forwarded-for");
	let middlewares = base_middlewares
		.sensitive_headers([header::AUTHORIZATION])
		.sensitive_request_headers([x_forwarded_for].into())
		.layer(axum::middleware::from_fn(request_spawn))
		.layer(
			TraceLayer::new_for_http()
				.make_span_with(tracing_span::<_>)
				.on_failure(DefaultOnFailure::new().level(Level::ERROR))
				.on_request(DefaultOnRequest::new().level(Level::TRACE))
				.on_response(DefaultOnResponse::new().level(Level::DEBUG)),
		)
		.layer(axum::middleware::from_fn(request_handle))
		.layer(cors_layer(server))
		.layer(DefaultBodyLimit::max(
			server
				.config
				.max_request_size
				.try_into()
				.expect("failed to convert max request size"),
		))
		.layer(CatchPanicLayer::custom(catch_panic_layer));

	#[cfg(any(feature = "zstd_compression", feature = "gzip_compression", feature = "brotli_compression"))]
	{
		Ok(routes::routes(&server.config)
			.layer(compression_layer(server))
			.layer(middlewares)
			.into_make_service())
	}
	#[cfg(not(any(feature = "zstd_compression", feature = "gzip_compression", feature = "brotli_compression")))]
	{
		Ok(routes::routes().layer(middlewares).into_make_service())
	}
}

#[tracing::instrument(skip_all, name = "spawn")]
async fn request_spawn(
	req: http::Request<axum::body::Body>, next: axum::middleware::Next,
) -> Result<axum::response::Response, StatusCode> {
	if services().globals.shutdown.load(atomic::Ordering::Relaxed) {
		return Err(StatusCode::SERVICE_UNAVAILABLE);
	}

	let fut = next.run(req);
	let task = tokio::spawn(fut);
	task.await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

#[tracing::instrument(skip_all, name = "handle")]
async fn request_handle(
	req: http::Request<axum::body::Body>, next: axum::middleware::Next,
) -> Result<axum::response::Response, StatusCode> {
	let method = req.method().clone();
	let uri = req.uri().clone();
	let result = next.run(req).await;
	request_result(&method, &uri, result)
}

fn request_result(
	method: &Method, uri: &Uri, result: axum::response::Response,
) -> Result<axum::response::Response, StatusCode> {
	request_result_log(method, uri, &result);
	match result.status() {
		StatusCode::METHOD_NOT_ALLOWED => request_result_403(method, uri, &result),
		_ => Ok(result),
	}
}

#[allow(clippy::unnecessary_wraps)]
fn request_result_403(
	_method: &Method, _uri: &Uri, result: &axum::response::Response,
) -> Result<axum::response::Response, StatusCode> {
	let error = UiaaResponse::MatrixError(RumaError {
		status_code: result.status(),
		body: ErrorBody::Standard {
			kind: ErrorKind::Unrecognized,
			message: "M_UNRECOGNIZED: Method not allowed for endpoint".to_owned(),
		},
	});

	Ok(RumaResponse(error).into_response())
}

fn request_result_log(method: &Method, uri: &Uri, result: &axum::response::Response) {
	let status = result.status();
	let reason = status.canonical_reason().unwrap_or("Unknown Reason");
	let code = status.as_u16();
	if status.is_server_error() {
		error!(method = ?method, uri = ?uri, "{code} {reason}");
	} else if status.is_client_error() {
		debug_error!(method = ?method, uri = ?uri, "{code} {reason}");
	} else if status.is_redirection() {
		debug!(method = ?method, uri = ?uri, "{code} {reason}");
	} else {
		trace!(method = ?method, uri = ?uri, "{code} {reason}");
	}
}

fn cors_layer(_server: &Server) -> CorsLayer {
	const METHODS: [Method; 6] = [
		Method::GET,
		Method::HEAD,
		Method::POST,
		Method::PUT,
		Method::DELETE,
		Method::OPTIONS,
	];

	let headers: [HeaderName; 5] = [
		header::ORIGIN,
		HeaderName::from_lowercase(b"x-requested-with").unwrap(),
		header::CONTENT_TYPE,
		header::ACCEPT,
		header::AUTHORIZATION,
	];

	CorsLayer::new()
		.allow_origin(cors::Any)
		.allow_methods(METHODS)
		.allow_headers(headers)
		.max_age(Duration::from_secs(86400))
}

#[cfg(any(feature = "zstd_compression", feature = "gzip_compression", feature = "brotli_compression"))]
fn compression_layer(server: &Server) -> tower_http::compression::CompressionLayer {
	let mut compression_layer = tower_http::compression::CompressionLayer::new();

	#[cfg(feature = "zstd_compression")]
	{
		if server.config.zstd_compression {
			compression_layer = compression_layer.zstd(true);
		} else {
			compression_layer = compression_layer.no_zstd();
		};
	};

	#[cfg(feature = "gzip_compression")]
	{
		if server.config.gzip_compression {
			compression_layer = compression_layer.gzip(true);
		} else {
			compression_layer = compression_layer.no_gzip();
		};
	};

	#[cfg(feature = "brotli_compression")]
	{
		if server.config.brotli_compression {
			compression_layer = compression_layer.br(true);
		} else {
			compression_layer = compression_layer.no_br();
		};
	};

	compression_layer
}

fn tracing_span<T>(request: &http::Request<T>) -> tracing::Span {
	let path = if let Some(path) = request.extensions().get::<MatchedPath>() {
		path.as_str()
	} else {
		request.uri().path()
	};

	tracing::info_span!("router:", %path)
}

/// Non-async initializations
fn init(args: clap::Args) -> Result<Server, Error> {
	let config = Config::new(args.config)?;

	#[cfg(feature = "sentry_telemetry")]
	let sentry_guard = if config.sentry {
		Some(init_sentry(&config))
	} else {
		None
	};

	let (tracing_reload_handle, tracing_flame_guard) = init_tracing(&config);

	config.check()?;

	info!(
		server_name = ?config.server_name,
		database_path = ?config.database_path,
		log_levels = ?config.log,
		"{}",
		env!("CARGO_PKG_VERSION"),
	);

	#[cfg(unix)]
	maximize_fd_limit().expect("Unable to increase maximum soft and hard file descriptor limit");

	Ok(Server {
		config,

		runtime: tokio::runtime::Builder::new_multi_thread()
			.enable_io()
			.enable_time()
			.thread_name("conduwuit:worker")
			.worker_threads(num_cpus::get())
			.build()
			.unwrap(),

		tracing_reload_handle,

		#[cfg(feature = "sentry_telemetry")]
		_sentry_guard: sentry_guard,
		_tracing_flame_guard: tracing_flame_guard,
	})
}

#[cfg(feature = "sentry_telemetry")]
fn init_sentry(config: &Config) -> sentry::ClientInitGuard {
	sentry::init((
		config
			.sentry_endpoint
			.as_ref()
			.expect("init_sentry should only be called if sentry is enabled and this is not None")
			.as_str(),
		sentry::ClientOptions {
			release: sentry::release_name!(),
			traces_sample_rate: config.sentry_traces_sample_rate,
			server_name: if config.sentry_send_server_name {
				Some(config.server_name.to_string().into())
			} else {
				None
			},
			..Default::default()
		},
	))
}

// We need to store a reload::Handle value, but can't name it's type explicitly
// because the S type parameter depends on the subscriber's previous layers. In
// our case, this includes unnameable 'impl Trait' types.
//
// This is fixed[1] in the unreleased tracing-subscriber from the master branch,
// which removes the S parameter. Unfortunately can't use it without pulling in
// a version of tracing that's incompatible with the rest of our deps.
//
// To work around this, we define an trait without the S paramter that forwards
// to the reload::Handle::reload method, and then store the handle as a trait
// object.
//
// [1]: https://github.com/tokio-rs/tracing/pull/1035/commits/8a87ea52425098d3ef8f56d92358c2f6c144a28f
trait ReloadHandle<L> {
	fn reload(&self, new_value: L) -> Result<(), reload::Error>;
}

impl<L, S> ReloadHandle<L> for reload::Handle<L, S> {
	fn reload(&self, new_value: L) -> Result<(), reload::Error> { reload::Handle::reload(self, new_value) }
}

struct LogLevelReloadHandlesInner {
	handles: Vec<Box<dyn ReloadHandle<EnvFilter> + Send + Sync>>,
}

/// Wrapper to allow reloading the filter on several several
/// [`tracing_subscriber::reload::Handle`]s at once, with the same value.
#[derive(Clone)]
struct LogLevelReloadHandles {
	inner: Arc<LogLevelReloadHandlesInner>,
}

impl LogLevelReloadHandles {
	fn new(handles: Vec<Box<dyn ReloadHandle<EnvFilter> + Send + Sync>>) -> LogLevelReloadHandles {
		LogLevelReloadHandles {
			inner: Arc::new(LogLevelReloadHandlesInner {
				handles,
			}),
		}
	}

	fn reload(&self, new_value: &EnvFilter) -> Result<(), reload::Error> {
		for handle in &self.inner.handles {
			handle.reload(new_value.clone())?;
		}
		Ok(())
	}
}

#[cfg(feature = "perf_measurements")]
type TracingFlameGuard = Option<tracing_flame::FlushGuard<io::BufWriter<fs::File>>>;
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

	#[cfg(feature = "tokio_console")]
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

			let (flame_layer, flame_guard) = tracing_flame::FlameLayer::with_file("./tracing.folded").unwrap();
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
	let flame_guard = ();

	tracing::subscriber::set_global_default(subscriber).unwrap();

	#[cfg(all(feature = "tokio_console", feature = "release_max_log_level"))]
	error!(
		"'tokio_console' feature and 'release_max_log_level' feature are incompatible, because console-subscriber \
		 needs access to trace-level events. 'release_max_log_level' must be disabled to use tokio-console."
	);

	(LogLevelReloadHandles::new(reload_handles), flame_guard)
}

// This is needed for opening lots of file descriptors, which tends to
// happen more often when using RocksDB and making lots of federation
// connections at startup. The soft limit is usually 1024, and the hard
// limit is usually 512000; I've personally seen it hit >2000.
//
// * https://www.freedesktop.org/software/systemd/man/systemd.exec.html#id-1.12.2.1.17.6
// * https://github.com/systemd/systemd/commit/0abf94923b4a95a7d89bc526efc84e7ca2b71741
#[cfg(unix)]
fn maximize_fd_limit() -> Result<(), nix::errno::Errno> {
	use nix::sys::resource::{getrlimit, setrlimit, Resource::RLIMIT_NOFILE as NOFILE};

	let (soft_limit, hard_limit) = getrlimit(NOFILE)?;
	if soft_limit < hard_limit {
		setrlimit(NOFILE, hard_limit, hard_limit)?;
		assert_eq!((hard_limit, hard_limit), getrlimit(NOFILE)?, "getrlimit != setrlimit");
		debug!(to = hard_limit, from = soft_limit, "Raised RLIMIT_NOFILE",);
	}

	Ok(())
}

#[allow(clippy::needless_pass_by_value)]
fn catch_panic_layer(err: Box<dyn Any + Send + 'static>) -> http::Response<http_body_util::Full<bytes::Bytes>> {
	let details = if let Some(s) = err.downcast_ref::<String>() {
		s.clone()
	} else if let Some(s) = err.downcast_ref::<&str>() {
		s.to_string()
	} else {
		"Unknown internal server error occurred.".to_owned()
	};

	let body = serde_json::json!({
		"errcode": "M_UNKNOWN",
		"error": "M_UNKNOWN: Internal server error occurred",
		"details": details,
	})
	.to_string();

	http::Response::builder()
		.status(StatusCode::INTERNAL_SERVER_ERROR)
		.header(header::CONTENT_TYPE, "application/json")
		.body(http_body_util::Full::from(body))
		.expect("Failed to create response for our panic catcher?")
}
