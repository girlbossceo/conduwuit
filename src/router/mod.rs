use std::{any::Any, io, sync::atomic, time::Duration};

use axum::{
	extract::{DefaultBodyLimit, MatchedPath},
	response::IntoResponse,
	Router,
};
use http::{
	header::{self, HeaderName, HeaderValue},
	Method, StatusCode, Uri,
};
use ruma::api::client::{
	error::{Error as RumaError, ErrorBody, ErrorKind},
	uiaa::UiaaResponse,
};
use tower::ServiceBuilder;
use tower_http::{
	catch_panic::CatchPanicLayer,
	cors::{self, CorsLayer},
	set_header::SetResponseHeaderLayer,
	trace::{DefaultOnFailure, DefaultOnRequest, DefaultOnResponse, TraceLayer},
	ServiceBuilderExt as _,
};
use tracing::{debug, error, trace, Level};

use super::{api::ruma_wrapper::RumaResponse, debug_error, services, utils::error::Result, Server};

mod routes;

pub(crate) async fn build(server: &Server) -> io::Result<axum::routing::IntoMakeService<Router>> {
	let base_middlewares = ServiceBuilder::new();
	#[cfg(feature = "sentry_telemetry")]
	let base_middlewares = base_middlewares.layer(sentry_tower::NewSentryLayer::<http::Request<_>>::new_from_top());

	let x_forwarded_for = HeaderName::from_static("x-forwarded-for");
	let permissions_policy = HeaderName::from_static("permissions-policy");
	let origin_agent_cluster = HeaderName::from_static("origin-agent-cluster"); // https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Origin-Agent-Cluster

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
		.layer(SetResponseHeaderLayer::if_not_present(
			origin_agent_cluster,
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
			permissions_policy,
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

/// Cross-Origin-Resource-Sharing header as defined by spec:
/// <https://spec.matrix.org/latest/client-server-api/#web-browser-clients>
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
