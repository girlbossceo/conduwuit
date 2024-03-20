#[cfg(unix)]
use std::fs::Permissions; // not unix specific, just only for UNIX sockets stuff and *nix container checks
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
#[cfg(unix)]
use std::path::Path; // not unix specific, just only for UNIX sockets stuff and *nix container checks
use std::{future::Future, io, net::SocketAddr, sync::atomic, time::Duration};

use axum::{
	extract::{DefaultBodyLimit, FromRequestParts, MatchedPath},
	response::IntoResponse,
	routing::{get, on, post, MethodFilter},
	Router,
};
use axum_server::{bind, bind_rustls, tls_rustls::RustlsConfig, Handle as ServerHandle};
#[cfg(feature = "axum_dual_protocol")]
use axum_server_dual_protocol::ServerExt;
use conduit::api::{client_server, server_server};
pub use conduit::*; // Re-export everything from the library crate
use either::Either::{Left, Right};
use figment::{
	providers::{Env, Format, Toml},
	Figment,
};
use http::{
	header::{self, HeaderName},
	Method, StatusCode, Uri,
};
#[cfg(unix)]
use hyperlocal::SocketIncoming;
use ruma::api::{
	client::{
		error::{Error as RumaError, ErrorBody, ErrorKind},
		uiaa::UiaaResponse,
	},
	IncomingRequest,
};
#[cfg(all(not(target_env = "msvc"), feature = "jemalloc"))]
use tikv_jemallocator::Jemalloc;
use tokio::{
	signal,
	sync::oneshot::{self, Sender},
	task::JoinSet,
};
use tower::ServiceBuilder;
use tower_http::{
	cors::{self, CorsLayer},
	trace::{DefaultOnFailure, TraceLayer},
	ServiceBuilderExt as _,
};
use tracing::{debug, error, info, warn, Level};
use tracing_subscriber::{prelude::*, EnvFilter};

#[cfg(all(not(target_env = "msvc"), feature = "jemalloc"))]
#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

#[tokio::main]
async fn main() {
	let args = clap::parse();

	// Initialize config
	let raw_config = if Env::var("CONDUIT_CONFIG").is_some() {
		Figment::new()
			.merge(
				Toml::file(Env::var("CONDUIT_CONFIG").expect(
					"The CONDUIT_CONFIG environment variable was set but appears to be invalid. This should be set to \
					 the path to a valid TOML file, an empty string (for compatibility), or removed/unset entirely.",
				))
				.nested(),
			)
			.merge(Env::prefixed("CONDUIT_").global())
	} else if args.config.is_some() {
		Figment::new()
			.merge(
				Toml::file(args.config.expect(
					"conduwuit config commandline argument was specified, but appears to be invalid. This should be \
					 set to the path of a valid TOML file.",
				))
				.nested(),
			)
			.merge(Env::prefixed("CONDUIT_").global())
	} else {
		Figment::new().merge(Env::prefixed("CONDUIT_").global())
	};

	let config = match raw_config.extract::<Config>() {
		Ok(s) => s,
		Err(e) => {
			eprintln!("It looks like your config is invalid. The following error occurred: {e}");
			return;
		},
	};

	if config.allow_jaeger {
		#[cfg(feature = "perf_measurements")]
		{
			opentelemetry::global::set_text_map_propagator(opentelemetry_jaeger::Propagator::new());
			let tracer = opentelemetry_jaeger::new_agent_pipeline()
				.with_auto_split_batch(true)
				.with_service_name("conduit")
				.install_batch(opentelemetry_sdk::runtime::Tokio)
				.unwrap();
			let telemetry = tracing_opentelemetry::layer().with_tracer(tracer);

			let filter_layer = match EnvFilter::try_new(&config.log) {
				Ok(s) => s,
				Err(e) => {
					eprintln!("It looks like your log config is invalid. The following error occurred: {e}");
					EnvFilter::try_new("warn").unwrap()
				},
			};

			let subscriber = tracing_subscriber::Registry::default().with(filter_layer).with(telemetry);
			tracing::subscriber::set_global_default(subscriber).unwrap();
		}
	} else if config.tracing_flame {
		#[cfg(feature = "perf_measurements")]
		{
			let registry = tracing_subscriber::Registry::default();
			let (flame_layer, _guard) = tracing_flame::FlameLayer::with_file("./tracing.folded").unwrap();
			let flame_layer = flame_layer.with_empty_samples(false);

			let filter_layer = EnvFilter::new("trace,h2=off");

			let subscriber = registry.with(filter_layer).with(flame_layer);
			tracing::subscriber::set_global_default(subscriber).unwrap();
		}
	} else {
		let registry = tracing_subscriber::Registry::default();
		let fmt_layer = tracing_subscriber::fmt::Layer::new();
		let filter_layer = match EnvFilter::try_new(&config.log) {
			Ok(s) => s,
			Err(e) => {
				eprintln!("It looks like your config is invalid. The following error occured while parsing it: {e}");
				EnvFilter::try_new("warn").unwrap()
			},
		};

		let subscriber = registry.with(filter_layer).with(fmt_layer);
		tracing::subscriber::set_global_default(subscriber).unwrap();
	}

	// This is needed for opening lots of file descriptors, which tends to
	// happen more often when using RocksDB and making lots of federation
	// connections at startup. The soft limit is usually 1024, and the hard
	// limit is usually 512000; I've personally seen it hit >2000.
	//
	// * https://www.freedesktop.org/software/systemd/man/systemd.exec.html#id-1.12.2.1.17.6
	// * https://github.com/systemd/systemd/commit/0abf94923b4a95a7d89bc526efc84e7ca2b71741
	#[cfg(unix)]
	maximize_fd_limit().expect("Unable to increase maximum soft and hard file descriptor limit");

	config.warn_deprecated();
	config.warn_unknown_key();

	// don't start if we're listening on both UNIX sockets and TCP at same time
	if config.is_dual_listening(&raw_config) {
		return;
	};

	info!("Loading database");
	let db_load_time = std::time::Instant::now();
	if let Err(error) = KeyValueDatabase::load_or_create(config).await {
		error!(?error, "The database couldn't be loaded or created");
		return;
	};
	info!("Database took {:?} to load", db_load_time.elapsed());

	let config = &services().globals.config;

	/* ad-hoc config validation/checks */

	if config.unix_socket_path.is_some() && !cfg!(unix) {
		error!(
			"UNIX socket support is only available on *nix platforms. Please remove \"unix_socket_path\" from your \
			 config."
		);
		return;
	}

	if config.address.is_loopback() && cfg!(unix) {
		debug!(
			"Found loopback listening address {}, running checks if we're in a container.",
			config.address
		);

		#[cfg(unix)]
		if Path::new("/proc/vz").exists() /* Guest */ && !Path::new("/proc/bz").exists()
		/* Host */
		{
			error!(
				"You are detected using OpenVZ with a loopback/localhost listening address of {}. If you are using \
				 OpenVZ for containers and you use NAT-based networking to communicate with the host and guest, this \
				 will NOT work. Please change this to \"0.0.0.0\". If this is expected, you can ignore.",
				config.address
			);
		}

		#[cfg(unix)]
		if Path::new("/.dockerenv").exists() {
			error!(
				"You are detected using Docker with a loopback/localhost listening address of {}. If you are using a \
				 reverse proxy on the host and require communication to conduwuit in the Docker container via \
				 NAT-based networking, this will NOT work. Please change this to \"0.0.0.0\". If this is expected, \
				 you can ignore.",
				config.address
			);
		}

		#[cfg(unix)]
		if Path::new("/run/.containerenv").exists() {
			error!(
				"You are detected using Podman with a loopback/localhost listening address of {}. If you are using a \
				 reverse proxy on the host and require communication to conduwuit in the Podman container via \
				 NAT-based networking, this will NOT work. Please change this to \"0.0.0.0\". If this is expected, \
				 you can ignore.",
				config.address
			);
		}
	}

	// rocksdb does not allow max_log_files to be 0
	if config.rocksdb_max_log_files == 0 && cfg!(feature = "rocksdb") {
		error!("When using RocksDB, rocksdb_max_log_files cannot be 0. Please set a value at least 1.");
		return;
	}

	// yeah, unless the user built a debug build hopefully for local testing only
	if config.server_name == "your.server.name" && !cfg!(debug_assertions) {
		error!("You must specify a valid server name for production usage of conduwuit.");
		return;
	}

	if cfg!(debug_assertions) {
		info!("Note: conduwuit was built without optimisations (i.e. debug build)");
	}

	// check if the user specified a registration token as `""`
	if config.registration_token == Some(String::new()) {
		error!("Registration token was specified but is empty (\"\")");
		return;
	}

	if config.max_request_size < 4096 {
		error!(?config.max_request_size, "Max request size is less than 4KB. Please increase it.");
	}

	// check if user specified valid IP CIDR ranges on startup
	for cidr in services().globals.ip_range_denylist() {
		let _ = ipaddress::IPAddress::parse(cidr).map_err(|e| error!("Error parsing specified IP CIDR range: {e}"));
	}

	if config.allow_registration
		&& !config.yes_i_am_very_very_sure_i_want_an_open_registration_server_prone_to_abuse
		&& config.registration_token.is_none()
	{
		error!(
			"!! You have `allow_registration` enabled without a token configured in your config which means you are \
			 allowing ANYONE to register on your conduwuit instance without any 2nd-step (e.g. registration token).\n
        If this is not the intended behaviour, please set a registration token with the `registration_token` config \
			 option.\n
        For security and safety reasons, conduwuit will shut down. If you are extra sure this is the desired behaviour \
			 you want, please set the following config option to true:
        `yes_i_am_very_very_sure_i_want_an_open_registration_server_prone_to_abuse`"
		);
		return;
	}

	if config.allow_registration
		&& config.yes_i_am_very_very_sure_i_want_an_open_registration_server_prone_to_abuse
		&& config.registration_token.is_none()
	{
		warn!(
			"Open registration is enabled via setting \
			 `yes_i_am_very_very_sure_i_want_an_open_registration_server_prone_to_abuse` and `allow_registration` to \
			 true without a registration token configured. You are expected to be aware of the risks now.\n
        If this is not the desired behaviour, please set a registration token."
		);
	}

	if config.allow_outgoing_presence && !config.allow_local_presence {
		error!("Outgoing presence requires allowing local presence. Please enable \"allow_local_presence\".");
		return;
	}

	if config.allow_outgoing_presence {
		warn!(
			"! Outgoing federated presence is not spec compliant due to relying on PDUs and EDUs combined.\nOutgoing \
			 presence will not be very reliable due to this and any issues with federated outgoing presence are very \
			 likely attributed to this issue.\nIncoming presence and local presence are unaffected."
		);
	}

	if config.url_preview_domain_contains_allowlist.contains(&"*".to_owned()) {
		warn!(
			"All URLs are allowed for URL previews via setting \"url_preview_domain_contains_allowlist\" to \"*\". \
			 This opens up significant attack surface to your server. You are expected to be aware of the risks by \
			 doing this."
		);
	}
	if config.url_preview_domain_explicit_allowlist.contains(&"*".to_owned()) {
		warn!(
			"All URLs are allowed for URL previews via setting \"url_preview_domain_explicit_allowlist\" to \"*\". \
			 This opens up significant attack surface to your server. You are expected to be aware of the risks by \
			 doing this."
		);
	}
	if config.url_preview_url_contains_allowlist.contains(&"*".to_owned()) {
		warn!(
			"All URLs are allowed for URL previews via setting \"url_preview_url_contains_allowlist\" to \"*\". This \
			 opens up significant attack surface to your server. You are expected to be aware of the risks by doing \
			 this."
		);
	}

	/* end ad-hoc config validation/checks */

	info!("Starting server");
	if let Err(e) = run_server().await {
		error!("Critical error starting server: {}", e);
	};

	// if server runs into critical error and shuts down, shut down the tracer
	// provider if jaegar is used. awaiting run_server() is a blocking call so
	// putting this after is fine, but not the other options above.
	#[cfg(feature = "perf_measurements")]
	if config.allow_jaeger {
		opentelemetry::global::shutdown_tracer_provider();
	}
}

async fn run_server() -> io::Result<()> {
	let config = &services().globals.config;

	let addrs = match &config.port.ports {
		Left(port) => {
			// Left is only 1 value, so make a vec with 1 value only
			let port_vec = [port];

			port_vec.iter().copied().map(|port| SocketAddr::from((config.address, *port))).collect::<Vec<_>>()
		},
		Right(ports) => ports.iter().copied().map(|port| SocketAddr::from((config.address, port))).collect::<Vec<_>>(),
	};

	let x_requested_with = HeaderName::from_static("x-requested-with");
	let x_forwarded_for = HeaderName::from_static("x-forwarded-for");

	let middlewares = ServiceBuilder::new()
		.sensitive_headers([header::AUTHORIZATION])
		.sensitive_request_headers([x_forwarded_for].into())
		.layer(axum::middleware::from_fn(spawn_task))
		.layer(
			TraceLayer::new_for_http()
				.make_span_with(|request: &http::Request<_>| {
					let path = if let Some(path) = request.extensions().get::<MatchedPath>() {
						path.as_str()
					} else {
						request.uri().path()
					};

					tracing::info_span!("http_request", %path)
				})
				.on_failure(DefaultOnFailure::new().level(Level::INFO)),
		)
		.layer(axum::middleware::from_fn(unrecognized_method))
		.layer(
			CorsLayer::new()
				.allow_origin(cors::Any)
				.allow_methods([
					Method::GET,
					Method::HEAD,
					Method::POST,
					Method::PUT,
					Method::DELETE,
					Method::OPTIONS,
				])
				.allow_headers([
					header::ORIGIN,
					x_requested_with,
					header::CONTENT_TYPE,
					header::ACCEPT,
					header::AUTHORIZATION,
				])
				.max_age(Duration::from_secs(86400)),
		)
		.layer(DefaultBodyLimit::max(
			config.max_request_size.try_into().expect("failed to convert max request size"),
		));

	let app;

	#[allow(clippy::unnecessary_operation)] // error[E0658]: attributes on expressions are experimental
	#[cfg(feature = "zstd_compression")]
	{
		app = if cfg!(feature = "zstd_compression") && config.zstd_compression {
			debug!("zstd body compression is enabled");
			routes().layer(middlewares.compression()).into_make_service()
		} else {
			routes().layer(middlewares).into_make_service()
		}
	};

	#[allow(clippy::unnecessary_operation)] // error[E0658]: attributes on expressions are experimental
	#[cfg(not(feature = "zstd_compression"))]
	{
		app = routes().layer(middlewares).into_make_service()
	};

	let handle = ServerHandle::new();

	#[allow(unused_variables)] // only rx is unused on non-*nix platforms
	let (tx, rx) = oneshot::channel::<()>();

	tokio::spawn(shutdown_signal(handle.clone(), tx));

	#[allow(unused_variables)] // path is unused on non-*nix platforms
	if let Some(path) = &config.unix_socket_path {
		#[cfg(unix)]
		{
			if path.exists() {
				warn!(
					"UNIX socket path {:#?} already exists (unclean shutdown?), attempting to remove it.",
					path.display()
				);
				tokio::fs::remove_file(&path).await?;
			}

			tokio::fs::create_dir_all(path.parent().unwrap()).await?;

			let socket_perms = config.unix_socket_perms.to_string();
			let octal_perms = u32::from_str_radix(&socket_perms, 8).unwrap();

			let listener = tokio::net::UnixListener::bind(path.clone())?;
			tokio::fs::set_permissions(path, Permissions::from_mode(octal_perms)).await.unwrap();
			let socket = SocketIncoming::from_listener(listener);

			#[cfg(feature = "systemd")]
			let _ = sd_notify::notify(true, &[sd_notify::NotifyState::Ready]);

			info!("Listening at {:?}", path);
			let server = hyper::Server::builder(socket).serve(app);
			let graceful = server.with_graceful_shutdown(async {
				rx.await.ok();
			});

			if let Err(e) = graceful.await {
				error!("Server error: {:?}", e);
			}
		}
	} else {
		match &config.tls {
			Some(tls) => {
				debug!(
					"Using direct TLS. Certificate path {} and certificate private key path {}",
					&tls.certs, &tls.key
				);
				info!(
					"Note: It is strongly recommended that you use a reverse proxy instead of running conduwuit \
					 directly with TLS."
				);
				let conf = RustlsConfig::from_pem_file(&tls.certs, &tls.key).await?;

				if cfg!(feature = "axum_dual_protocol") {
					info!(
						"conduwuit was built with axum_dual_protocol feature to listen on both HTTP and HTTPS. This \
						 will only take affect if `dual_protocol` is enabled in `[global.tls]`"
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
						join_set.spawn(bind_rustls(*addr, conf.clone()).handle(handle.clone()).serve(app.clone()));
					}
				}

				#[cfg(feature = "systemd")]
				let _ = sd_notify::notify(true, &[sd_notify::NotifyState::Ready]);

				if cfg!(feature = "axum_dual_protocol") && tls.dual_protocol {
					warn!(
						"Listening on {:?} with TLS certificate {} and supporting plain text (HTTP) connections too \
						 (insecure!)",
						addrs, &tls.certs
					);
				} else {
					info!("Listening on {:?} with TLS certificate {}", addrs, &tls.certs);
				}

				join_set.join_next().await;
			},
			None => {
				let mut join_set = JoinSet::new();
				for addr in &addrs {
					join_set.spawn(bind(*addr).handle(handle.clone()).serve(app.clone()));
				}

				#[cfg(feature = "systemd")]
				let _ = sd_notify::notify(true, &[sd_notify::NotifyState::Ready]);

				info!("Listening on {:?}", addrs);
				join_set.join_next().await;
			},
		}
	}

	Ok(())
}

async fn spawn_task<B: Send + 'static>(
	req: http::Request<B>, next: axum::middleware::Next<B>,
) -> std::result::Result<axum::response::Response, StatusCode> {
	if services().globals.shutdown.load(atomic::Ordering::Relaxed) {
		return Err(StatusCode::SERVICE_UNAVAILABLE);
	}
	tokio::spawn(next.run(req)).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn unrecognized_method<B: Send + 'static>(
	req: http::Request<B>, next: axum::middleware::Next<B>,
) -> std::result::Result<axum::response::Response, StatusCode> {
	let method = req.method().clone();
	let uri = req.uri().clone();
	let inner = next.run(req).await;
	if inner.status() == StatusCode::METHOD_NOT_ALLOWED {
		warn!("Method not allowed: {method} {uri}");
		return Ok(RumaResponse(UiaaResponse::MatrixError(RumaError {
			body: ErrorBody::Standard {
				kind: ErrorKind::Unrecognized,
				message: "M_UNRECOGNIZED: Unrecognized request".to_owned(),
			},
			status_code: StatusCode::METHOD_NOT_ALLOWED,
		}))
		.into_response());
	}
	Ok(inner)
}

fn routes() -> Router {
	Router::new()
		.ruma_route(client_server::get_supported_versions_route)
		.ruma_route(client_server::get_register_available_route)
		.ruma_route(client_server::register_route)
		.ruma_route(client_server::get_login_types_route)
		.ruma_route(client_server::login_route)
		.ruma_route(client_server::whoami_route)
		.ruma_route(client_server::logout_route)
		.ruma_route(client_server::logout_all_route)
		.ruma_route(client_server::change_password_route)
		.ruma_route(client_server::deactivate_route)
		.ruma_route(client_server::third_party_route)
		.ruma_route(client_server::request_3pid_management_token_via_email_route)
		.ruma_route(client_server::request_3pid_management_token_via_msisdn_route)
		.ruma_route(client_server::get_capabilities_route)
		.ruma_route(client_server::get_pushrules_all_route)
		.ruma_route(client_server::set_pushrule_route)
		.ruma_route(client_server::get_pushrule_route)
		.ruma_route(client_server::set_pushrule_enabled_route)
		.ruma_route(client_server::get_pushrule_enabled_route)
		.ruma_route(client_server::get_pushrule_actions_route)
		.ruma_route(client_server::set_pushrule_actions_route)
		.ruma_route(client_server::delete_pushrule_route)
		.ruma_route(client_server::get_room_event_route)
		.ruma_route(client_server::get_room_aliases_route)
		.ruma_route(client_server::get_filter_route)
		.ruma_route(client_server::create_filter_route)
		.ruma_route(client_server::set_global_account_data_route)
		.ruma_route(client_server::set_room_account_data_route)
		.ruma_route(client_server::get_global_account_data_route)
		.ruma_route(client_server::get_room_account_data_route)
		.ruma_route(client_server::set_displayname_route)
		.ruma_route(client_server::get_displayname_route)
		.ruma_route(client_server::set_avatar_url_route)
		.ruma_route(client_server::get_avatar_url_route)
		.ruma_route(client_server::get_profile_route)
		.ruma_route(client_server::set_presence_route)
		.ruma_route(client_server::get_presence_route)
		.ruma_route(client_server::upload_keys_route)
		.ruma_route(client_server::get_keys_route)
		.ruma_route(client_server::claim_keys_route)
		.ruma_route(client_server::create_backup_version_route)
		.ruma_route(client_server::update_backup_version_route)
		.ruma_route(client_server::delete_backup_version_route)
		.ruma_route(client_server::get_latest_backup_info_route)
		.ruma_route(client_server::get_backup_info_route)
		.ruma_route(client_server::add_backup_keys_route)
		.ruma_route(client_server::add_backup_keys_for_room_route)
		.ruma_route(client_server::add_backup_keys_for_session_route)
		.ruma_route(client_server::delete_backup_keys_for_room_route)
		.ruma_route(client_server::delete_backup_keys_for_session_route)
		.ruma_route(client_server::delete_backup_keys_route)
		.ruma_route(client_server::get_backup_keys_for_room_route)
		.ruma_route(client_server::get_backup_keys_for_session_route)
		.ruma_route(client_server::get_backup_keys_route)
		.ruma_route(client_server::set_read_marker_route)
		.ruma_route(client_server::create_receipt_route)
		.ruma_route(client_server::create_typing_event_route)
		.ruma_route(client_server::create_room_route)
		.ruma_route(client_server::redact_event_route)
		.ruma_route(client_server::report_event_route)
		.ruma_route(client_server::create_alias_route)
		.ruma_route(client_server::delete_alias_route)
		.ruma_route(client_server::get_alias_route)
		.ruma_route(client_server::join_room_by_id_route)
		.ruma_route(client_server::join_room_by_id_or_alias_route)
		.ruma_route(client_server::joined_members_route)
		.ruma_route(client_server::leave_room_route)
		.ruma_route(client_server::forget_room_route)
		.ruma_route(client_server::joined_rooms_route)
		.ruma_route(client_server::kick_user_route)
		.ruma_route(client_server::ban_user_route)
		.ruma_route(client_server::unban_user_route)
		.ruma_route(client_server::invite_user_route)
		.ruma_route(client_server::set_room_visibility_route)
		.ruma_route(client_server::get_room_visibility_route)
		.ruma_route(client_server::get_public_rooms_route)
		.ruma_route(client_server::get_public_rooms_filtered_route)
		.ruma_route(client_server::search_users_route)
		.ruma_route(client_server::get_member_events_route)
		.ruma_route(client_server::get_protocols_route)
		.ruma_route(client_server::send_message_event_route)
		.ruma_route(client_server::send_state_event_for_key_route)
		.ruma_route(client_server::get_state_events_route)
		.ruma_route(client_server::get_state_events_for_key_route)
		// Ruma doesn't have support for multiple paths for a single endpoint yet, and these routes
		// share one Ruma request / response type pair with {get,send}_state_event_for_key_route
		.route(
			"/_matrix/client/r0/rooms/:room_id/state/:event_type",
			get(client_server::get_state_events_for_empty_key_route)
				.put(client_server::send_state_event_for_empty_key_route),
		)
		.route(
			"/_matrix/client/v3/rooms/:room_id/state/:event_type",
			get(client_server::get_state_events_for_empty_key_route)
				.put(client_server::send_state_event_for_empty_key_route),
		)
		// These two endpoints allow trailing slashes
		.route(
			"/_matrix/client/r0/rooms/:room_id/state/:event_type/",
			get(client_server::get_state_events_for_empty_key_route)
				.put(client_server::send_state_event_for_empty_key_route),
		)
		.route(
			"/_matrix/client/v3/rooms/:room_id/state/:event_type/",
			get(client_server::get_state_events_for_empty_key_route)
				.put(client_server::send_state_event_for_empty_key_route),
		)
		.ruma_route(client_server::sync_events_route)
		.ruma_route(client_server::sync_events_v4_route)
		.ruma_route(client_server::get_context_route)
		.ruma_route(client_server::get_message_events_route)
		.ruma_route(client_server::search_events_route)
		.ruma_route(client_server::turn_server_route)
		.ruma_route(client_server::send_event_to_device_route)
		.ruma_route(client_server::get_media_config_route)
		.ruma_route(client_server::get_media_preview_route)
		.ruma_route(client_server::create_content_route)
		// legacy v1 media routes
		.route(
			"/_matrix/media/v1/url_preview",
			get(client_server::get_media_preview_v1_route)
		)
		.route(
			"/_matrix/media/v1/config",
			get(client_server::get_media_config_v1_route)
		)
		.route(
			"/_matrix/media/v1/upload",
			post(client_server::create_content_v1_route)
		)
		.route(
			"/_matrix/media/v1/download/:server_name/:media_id",
			get(client_server::get_content_v1_route)
		)
		.route(
			"/_matrix/media/v1/download/:server_name/:media_id/:file_name",
			get(client_server::get_content_as_filename_v1_route)
		)
		.route(
			"/_matrix/media/v1/thumbnail/:server_name/:media_id",
			get(client_server::get_content_thumbnail_v1_route)
		)
		.ruma_route(client_server::get_content_route)
		.ruma_route(client_server::get_content_as_filename_route)
		.ruma_route(client_server::get_content_thumbnail_route)
		.ruma_route(client_server::get_devices_route)
		.ruma_route(client_server::get_device_route)
		.ruma_route(client_server::update_device_route)
		.ruma_route(client_server::delete_device_route)
		.ruma_route(client_server::delete_devices_route)
		.ruma_route(client_server::get_tags_route)
		.ruma_route(client_server::update_tag_route)
		.ruma_route(client_server::delete_tag_route)
		.ruma_route(client_server::upload_signing_keys_route)
		.ruma_route(client_server::upload_signatures_route)
		.ruma_route(client_server::get_key_changes_route)
		.ruma_route(client_server::get_pushers_route)
		.ruma_route(client_server::set_pushers_route)
		// .ruma_route(client_server::third_party_route)
		.ruma_route(client_server::upgrade_room_route)
		.ruma_route(client_server::get_threads_route)
		.ruma_route(client_server::get_relating_events_with_rel_type_and_event_type_route)
		.ruma_route(client_server::get_relating_events_with_rel_type_route)
		.ruma_route(client_server::get_relating_events_route)
		.ruma_route(client_server::get_hierarchy_route)
		.ruma_route(server_server::get_server_version_route)
		.route("/_matrix/key/v2/server", get(server_server::get_server_keys_route))
		.route(
			"/_matrix/key/v2/server/:key_id",
			get(server_server::get_server_keys_deprecated_route),
		)
		.ruma_route(server_server::get_public_rooms_route)
		.ruma_route(server_server::get_public_rooms_filtered_route)
		.ruma_route(server_server::send_transaction_message_route)
		.ruma_route(server_server::get_event_route)
		.ruma_route(server_server::get_backfill_route)
		.ruma_route(server_server::get_missing_events_route)
		.ruma_route(server_server::get_event_authorization_route)
		.ruma_route(server_server::get_room_state_route)
		.ruma_route(server_server::get_room_state_ids_route)
		.ruma_route(server_server::create_join_event_template_route)
		.ruma_route(server_server::create_join_event_v1_route)
		.ruma_route(server_server::create_join_event_v2_route)
		.ruma_route(server_server::create_invite_route)
		.ruma_route(server_server::get_devices_route)
		.ruma_route(server_server::get_room_information_route)
		.ruma_route(server_server::get_profile_information_route)
		.ruma_route(server_server::get_keys_route)
		.ruma_route(server_server::claim_keys_route)
        .ruma_route(server_server::get_hierarchy_route)
		.route("/_matrix/client/r0/rooms/:room_id/initialSync", get(initial_sync))
		.route("/_matrix/client/v3/rooms/:room_id/initialSync", get(initial_sync))
		.route("/client/server.json", get(client_server::syncv3_client_server_json))
		.route("/.well-known/matrix/client", get(client_server::well_known_client_route))
		.route("/.well-known/matrix/server", get(server_server::well_known_server_route))
		.route("/", get(it_works))
		.fallback(not_found)
}

async fn shutdown_signal(handle: ServerHandle, tx: Sender<()>) -> Result<()> {
	let ctrl_c = async {
		signal::ctrl_c().await.expect("failed to install Ctrl+C handler");
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
		_ = ctrl_c => { sig = "Ctrl+C"; },
		_ = terminate => { sig = "SIGTERM"; },
	}

	#[cfg(not(unix))]
	tokio::select! {
		_ = ctrl_c => { sig = "Ctrl+C"; },
	}

	warn!("Received {}, shutting down...", sig);
	let shutdown_time_elapsed = tokio::time::Instant::now();
	handle.graceful_shutdown(Some(Duration::from_secs(180)));

	services().globals.shutdown();

	#[cfg(feature = "systemd")]
	let _ = sd_notify::notify(true, &[sd_notify::NotifyState::Stopping]);

	tx.send(()).expect(
		"failed sending shutdown transaction to oneshot channel (this is unlikely a conduwuit bug and more so your \
		 system may not be in an okay/ideal state.)",
	);

	if shutdown_time_elapsed.elapsed() >= Duration::from_secs(60) && cfg!(feature = "systemd") {
		warn!(
			"Still shutting down after 60 seconds since receiving shutdown signal, asking systemd for more time (+120 \
			 seconds). Remaining connections: {}",
			handle.connection_count()
		);

		#[cfg(feature = "systemd")]
		let _ = sd_notify::notify(true, &[sd_notify::NotifyState::ExtendTimeoutUsec(120)]);
	}

	warn!("Time took to shutdown: {:?} seconds", shutdown_time_elapsed.elapsed());

	Ok(())
}

async fn not_found(uri: Uri) -> impl IntoResponse {
	warn!("Not found: {uri}");
	Error::BadRequest(ErrorKind::Unrecognized, "Unrecognized request")
}

async fn initial_sync(_uri: Uri) -> impl IntoResponse {
	Error::BadRequest(ErrorKind::GuestAccessForbidden, "Guest access not implemented")
}

async fn it_works() -> &'static str { "hewwo from conduwuit woof!" }

trait RouterExt {
	fn ruma_route<H, T>(self, handler: H) -> Self
	where
		H: RumaHandler<T>,
		T: 'static;
}

impl RouterExt for Router {
	fn ruma_route<H, T>(self, handler: H) -> Self
	where
		H: RumaHandler<T>,
		T: 'static,
	{
		handler.add_to_router(self)
	}
}

pub trait RumaHandler<T> {
	// Can't transform to a handler without boxing or relying on the nightly-only
	// impl-trait-in-traits feature. Moving a small amount of extra logic into the
	// trait allows bypassing both.
	fn add_to_router(self, router: Router) -> Router;
}

macro_rules! impl_ruma_handler {
    ( $($ty:ident),* $(,)? ) => {
        #[axum::async_trait]
        #[allow(non_snake_case)]
        impl<Req, E, F, Fut, $($ty,)*> RumaHandler<($($ty,)* Ruma<Req>,)> for F
        where
            Req: IncomingRequest + Send + 'static,
            F: FnOnce($($ty,)* Ruma<Req>) -> Fut + Clone + Send + 'static,
            Fut: Future<Output = Result<Req::OutgoingResponse, E>>
                + Send,
            E: IntoResponse,
            $( $ty: FromRequestParts<()> + Send + 'static, )*
        {
            fn add_to_router(self, mut router: Router) -> Router {
                let meta = Req::METADATA;
                let method_filter = method_to_filter(meta.method);

                for path in meta.history.all_paths() {
                    let handler = self.clone();

                    router = router.route(path, on(method_filter, |$( $ty: $ty, )* req| async move {
                        handler($($ty,)* req).await.map(RumaResponse)
                    }))
                }

                router
            }
        }
    };
}

impl_ruma_handler!();
impl_ruma_handler!(T1);
impl_ruma_handler!(T1, T2);
impl_ruma_handler!(T1, T2, T3);
impl_ruma_handler!(T1, T2, T3, T4);
impl_ruma_handler!(T1, T2, T3, T4, T5);
impl_ruma_handler!(T1, T2, T3, T4, T5, T6);
impl_ruma_handler!(T1, T2, T3, T4, T5, T6, T7);
impl_ruma_handler!(T1, T2, T3, T4, T5, T6, T7, T8);

fn method_to_filter(method: Method) -> MethodFilter {
	match method {
		Method::DELETE => MethodFilter::DELETE,
		Method::GET => MethodFilter::GET,
		Method::HEAD => MethodFilter::HEAD,
		Method::OPTIONS => MethodFilter::OPTIONS,
		Method::PATCH => MethodFilter::PATCH,
		Method::POST => MethodFilter::POST,
		Method::PUT => MethodFilter::PUT,
		Method::TRACE => MethodFilter::TRACE,
		m => panic!("Unsupported HTTP method: {m:?}"),
	}
}

#[cfg(unix)]
#[tracing::instrument(err)]
fn maximize_fd_limit() -> Result<(), nix::errno::Errno> {
	use nix::sys::resource::{getrlimit, setrlimit, Resource};

	let res = Resource::RLIMIT_NOFILE;

	let (soft_limit, hard_limit) = getrlimit(res)?;

	debug!("Current nofile soft limit: {soft_limit}");

	setrlimit(res, hard_limit, hard_limit)?;

	debug!("Increased nofile soft limit to {hard_limit}");

	Ok(())
}
