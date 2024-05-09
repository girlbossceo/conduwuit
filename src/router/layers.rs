use std::{any::Any, io, sync::Arc, time::Duration};

use axum::{
	extract::{DefaultBodyLimit, MatchedPath},
	Router,
};
use conduit::Server;
use http::{
	header::{self, HeaderName},
	HeaderValue, Method, StatusCode,
};
use tower::ServiceBuilder;
use tower_http::{
	catch_panic::CatchPanicLayer,
	cors::{self, CorsLayer},
	set_header::SetResponseHeaderLayer,
	trace::{DefaultOnFailure, DefaultOnRequest, DefaultOnResponse, TraceLayer},
	ServiceBuilderExt as _,
};
use tracing::Level;

use crate::{request, router};

pub(crate) fn build(server: &Arc<Server>) -> io::Result<axum::routing::IntoMakeService<Router>> {
	let layers = ServiceBuilder::new();

	#[cfg(feature = "sentry_telemetry")]
	let layers = layers.layer(sentry_tower::NewSentryLayer::<http::Request<_>>::new_from_top());

	#[cfg(any(feature = "zstd_compression", feature = "gzip_compression", feature = "brotli_compression"))]
	let layers = layers.layer(compression_layer(server));

	let layers = layers
		.sensitive_headers([header::AUTHORIZATION])
		.sensitive_request_headers([HeaderName::from_static("x-forwarded-for")].into())
		.layer(axum::middleware::from_fn_with_state(Arc::clone(server), request::spawn))
		.layer(
			TraceLayer::new_for_http()
				.make_span_with(tracing_span::<_>)
				.on_failure(DefaultOnFailure::new().level(Level::ERROR))
				.on_request(DefaultOnRequest::new().level(Level::TRACE))
				.on_response(DefaultOnResponse::new().level(Level::DEBUG)),
		)
		.layer(axum::middleware::from_fn_with_state(Arc::clone(server), request::handle))
		.layer(SetResponseHeaderLayer::if_not_present(
			HeaderName::from_static("origin-agent-cluster"), // https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Origin-Agent-Cluster
			HeaderValue::from_static("?1"),
		))
		.layer(SetResponseHeaderLayer::if_not_present(
			header::X_CONTENT_TYPE_OPTIONS,
			HeaderValue::from_static("nosniff"),
		))
		.layer(SetResponseHeaderLayer::if_not_present(
			header::X_XSS_PROTECTION,
			HeaderValue::from_static("0"),
		))
		.layer(SetResponseHeaderLayer::if_not_present(
			header::X_FRAME_OPTIONS,
			HeaderValue::from_static("DENY"),
		))
		.layer(SetResponseHeaderLayer::if_not_present(
			HeaderName::from_static("permissions-policy"),
			HeaderValue::from_static("interest-cohort=(),browsing-topics=()"),
		))
		.layer(SetResponseHeaderLayer::if_not_present(
			header::CONTENT_SECURITY_POLICY,
			HeaderValue::from_static(
				"sandbox; default-src 'none'; font-src 'none'; script-src 'none'; plugin-types application/pdf; \
				 style-src 'unsafe-inline'; object-src 'self'; frame-ancesors 'none';",
			),
		))
		.layer(cors_layer(server))
		.layer(body_limit_layer(server))
		.layer(CatchPanicLayer::custom(catch_panic));

	let routes = router::build(server);
	let layers = routes.layer(layers);

	Ok(layers.into_make_service())
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

fn cors_layer(_server: &Server) -> CorsLayer {
	const METHODS: [Method; 7] = [
		Method::GET,
		Method::HEAD,
		Method::PATCH,
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

fn body_limit_layer(server: &Server) -> DefaultBodyLimit {
	DefaultBodyLimit::max(
		server
			.config
			.max_request_size
			.try_into()
			.expect("failed to convert max request size"),
	)
}

#[allow(clippy::needless_pass_by_value)]
#[tracing::instrument(skip_all)]
fn catch_panic(err: Box<dyn Any + Send + 'static>) -> http::Response<http_body_util::Full<bytes::Bytes>> {
	conduit_service::services()
		.server
		.requests_panic
		.fetch_add(1, std::sync::atomic::Ordering::Release);

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

fn tracing_span<T>(request: &http::Request<T>) -> tracing::Span {
	let path = if let Some(path) = request.extensions().get::<MatchedPath>() {
		path.as_str()
	} else {
		request.uri().path()
	};

	tracing::info_span!("router:", %path)
}
