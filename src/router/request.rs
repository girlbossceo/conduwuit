use std::sync::{atomic::Ordering, Arc};

use axum::{extract::State, response::IntoResponse};
use conduit::{debug_error, debug_warn, defer, Result, RumaResponse, Server};
use http::{Method, StatusCode, Uri};
use ruma::api::client::{
	error::{Error as RumaError, ErrorBody, ErrorKind},
	uiaa::UiaaResponse,
};
use tracing::{debug, error, trace};

#[tracing::instrument(skip_all)]
pub(crate) async fn spawn(
	State(server): State<Arc<Server>>, req: http::Request<axum::body::Body>, next: axum::middleware::Next,
) -> Result<axum::response::Response, StatusCode> {
	if server.interrupt.load(Ordering::Relaxed) {
		debug_warn!("unavailable pending shutdown");
		return Err(StatusCode::SERVICE_UNAVAILABLE);
	}

	let active = server.requests_spawn_active.fetch_add(1, Ordering::Relaxed);
	trace!(active, "enter");
	defer! {{
		let active = server.requests_spawn_active.fetch_sub(1, Ordering::Relaxed);
		let finished = server.requests_spawn_finished.fetch_add(1, Ordering::Relaxed);
		trace!(active, finished, "leave");
	}};

	let fut = next.run(req);
	let task = server.runtime().spawn(fut);
	task.await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

#[tracing::instrument(skip_all, name = "handle")]
pub(crate) async fn handle(
	State(server): State<Arc<Server>>, req: http::Request<axum::body::Body>, next: axum::middleware::Next,
) -> Result<axum::response::Response, StatusCode> {
	if server.interrupt.load(Ordering::Relaxed) {
		debug_warn!(
			method = %req.method(),
			uri = %req.uri(),
			"unavailable pending shutdown"
		);

		return Err(StatusCode::SERVICE_UNAVAILABLE);
	}

	let active = server
		.requests_handle_active
		.fetch_add(1, Ordering::Relaxed);
	trace!(active, "enter");
	defer! {{
		let active = server.requests_handle_active.fetch_sub(1, Ordering::Relaxed);
		let finished = server.requests_handle_finished.fetch_add(1, Ordering::Relaxed);
		trace!(active, finished, "leave");
	}};

	let method = req.method().clone();
	let uri = req.uri().clone();
	let result = next.run(req).await;
	handle_result(&method, &uri, result)
}

fn handle_result(
	method: &Method, uri: &Uri, result: axum::response::Response,
) -> Result<axum::response::Response, StatusCode> {
	handle_result_log(method, uri, &result);
	match result.status() {
		StatusCode::METHOD_NOT_ALLOWED => handle_result_403(method, uri, &result),
		_ => Ok(result),
	}
}

#[allow(clippy::unnecessary_wraps)]
fn handle_result_403(
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

fn handle_result_log(method: &Method, uri: &Uri, result: &axum::response::Response) {
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
