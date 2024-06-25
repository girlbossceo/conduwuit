use std::sync::{atomic::Ordering, Arc};

use axum::{
	extract::State,
	response::{IntoResponse, Response},
};
use conduit::{debug, debug_error, debug_warn, defer, error, trace, Error, Result, Server};
use http::{Method, StatusCode, Uri};
use ruma::api::client::error::{Error as RumaError, ErrorBody, ErrorKind};

#[tracing::instrument(skip_all, level = "debug")]
pub(crate) async fn spawn(
	State(server): State<Arc<Server>>, req: http::Request<axum::body::Body>, next: axum::middleware::Next,
) -> Result<Response, StatusCode> {
	if !server.running() {
		debug_warn!("unavailable pending shutdown");
		return Err(StatusCode::SERVICE_UNAVAILABLE);
	}

	let active = server
		.metrics
		.requests_spawn_active
		.fetch_add(1, Ordering::Relaxed);
	trace!(active, "enter");
	defer! {{
		let active = server.metrics.requests_spawn_active.fetch_sub(1, Ordering::Relaxed);
		let finished = server.metrics.requests_spawn_finished.fetch_add(1, Ordering::Relaxed);
		trace!(active, finished, "leave");
	}};

	let fut = next.run(req);
	let task = server.runtime().spawn(fut);
	task.await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

#[tracing::instrument(skip_all, name = "handle")]
pub(crate) async fn handle(
	State(server): State<Arc<Server>>, req: http::Request<axum::body::Body>, next: axum::middleware::Next,
) -> Result<Response, StatusCode> {
	if !server.running() {
		debug_warn!(
			method = %req.method(),
			uri = %req.uri(),
			"unavailable pending shutdown"
		);

		return Err(StatusCode::SERVICE_UNAVAILABLE);
	}

	let active = server
		.metrics
		.requests_handle_active
		.fetch_add(1, Ordering::Relaxed);
	trace!(active, "enter");
	defer! {{
		let active = server.metrics.requests_handle_active.fetch_sub(1, Ordering::Relaxed);
		let finished = server.metrics.requests_handle_finished.fetch_add(1, Ordering::Relaxed);
		trace!(active, finished, "leave");
	}};

	let method = req.method().clone();
	let uri = req.uri().clone();
	let result = next.run(req).await;
	handle_result(&method, &uri, result)
}

fn handle_result(method: &Method, uri: &Uri, result: Response) -> Result<Response, StatusCode> {
	handle_result_log(method, uri, &result);
	match result.status() {
		StatusCode::METHOD_NOT_ALLOWED => handle_result_405(method, uri, &result),
		_ => Ok(result),
	}
}

fn handle_result_405(_method: &Method, _uri: &Uri, result: &Response) -> Result<Response, StatusCode> {
	let error = Error::RumaError(RumaError {
		status_code: result.status(),
		body: ErrorBody::Standard {
			kind: ErrorKind::Unrecognized,
			message: "M_UNRECOGNIZED: Method not allowed for endpoint".to_owned(),
		},
	});

	Ok(error.into_response())
}

fn handle_result_log(method: &Method, uri: &Uri, result: &Response) {
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
