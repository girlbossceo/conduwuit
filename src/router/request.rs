use std::sync::{atomic::Ordering, Arc};

use axum::{
	extract::State,
	response::{IntoResponse, Response},
};
use conduwuit::{debug, debug_error, debug_warn, defer, err, error, trace, Result};
use conduwuit_service::Services;
use http::{Method, StatusCode, Uri};

#[tracing::instrument(
	parent = None,
	level = "trace",
	skip_all,
	fields(
		handled = %services
			.server
			.metrics
			.requests_spawn_finished
			.fetch_add(1, Ordering::Relaxed),
		active = %services
			.server
			.metrics
			.requests_spawn_active
			.fetch_add(1, Ordering::Relaxed),
	)
)]
pub(crate) async fn spawn(
	State(services): State<Arc<Services>>,
	req: http::Request<axum::body::Body>,
	next: axum::middleware::Next,
) -> Result<Response, StatusCode> {
	let server = &services.server;

	#[cfg(debug_assertions)]
	defer! {{
		_ = server
			.metrics
			.requests_spawn_active
			.fetch_sub(1, Ordering::Relaxed);
	}};

	if !server.running() {
		debug_warn!("unavailable pending shutdown");
		return Err(StatusCode::SERVICE_UNAVAILABLE);
	}

	let fut = next.run(req);
	let task = server.runtime().spawn(fut);
	task.await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

#[tracing::instrument(
	level = "debug",
	skip_all,
	fields(
		handled = %services
			.server
			.metrics
			.requests_handle_finished
			.fetch_add(1, Ordering::Relaxed),
		active = %services
			.server
			.metrics
			.requests_handle_active
			.fetch_add(1, Ordering::Relaxed),
	)
)]
pub(crate) async fn handle(
	State(services): State<Arc<Services>>,
	req: http::Request<axum::body::Body>,
	next: axum::middleware::Next,
) -> Result<Response, StatusCode> {
	let server = &services.server;

	#[cfg(debug_assertions)]
	defer! {{
		_ = server
			.metrics
			.requests_handle_active
			.fetch_sub(1, Ordering::Relaxed);
	}};

	if !server.running() {
		debug_warn!(
			method = %req.method(),
			uri = %req.uri(),
			"unavailable pending shutdown"
		);

		return Err(StatusCode::SERVICE_UNAVAILABLE);
	}

	let uri = req.uri().clone();
	let method = req.method().clone();
	let result = next.run(req).await;
	handle_result(&method, &uri, result)
}

fn handle_result(method: &Method, uri: &Uri, result: Response) -> Result<Response, StatusCode> {
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

	if status == StatusCode::METHOD_NOT_ALLOWED {
		return Ok(err!(Request(Unrecognized("Method Not Allowed"))).into_response());
	}

	Ok(result)
}
