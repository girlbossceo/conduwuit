use std::{
	fmt::Debug,
	sync::{atomic::Ordering, Arc},
};

use axum::{
	extract::State,
	response::{IntoResponse, Response},
};
use conduwuit::{debug, debug_error, debug_warn, err, error, trace, Result};
use conduwuit_service::Services;
use http::{Method, StatusCode, Uri};
use tracing::Span;

#[tracing::instrument(
	name = "request",
	level = "debug",
	skip_all,
	fields(
		active = %services
			.server
			.metrics
			.requests_handle_active
			.fetch_add(1, Ordering::Relaxed),
		handled = %services
			.server
			.metrics
			.requests_handle_finished
			.load(Ordering::Relaxed),
	)
)]
pub(crate) async fn handle(
	State(services): State<Arc<Services>>,
	req: http::Request<axum::body::Body>,
	next: axum::middleware::Next,
) -> Result<Response, StatusCode> {
	#[cfg(debug_assertions)]
	conduwuit::defer! {{
		_ = services.server
			.metrics
			.requests_handle_finished
			.fetch_add(1, Ordering::Relaxed);
		_ = services.server
			.metrics
			.requests_handle_active
			.fetch_sub(1, Ordering::Relaxed);
	}};

	if !services.server.running() {
		debug_warn!(
			method = %req.method(),
			uri = %req.uri(),
			"unavailable pending shutdown"
		);

		return Err(StatusCode::SERVICE_UNAVAILABLE);
	}

	let uri = req.uri().clone();
	let method = req.method().clone();
	let services_ = services.clone();
	let parent = Span::current();
	let task = services.server.runtime().spawn(async move {
		tokio::select! {
			response = execute(&services_, req, next, parent) => response,
			() = services_.server.until_shutdown() =>
				StatusCode::SERVICE_UNAVAILABLE.into_response(),
		}
	});

	task.await
		.map_err(unhandled)
		.and_then(move |result| handle_result(&method, &uri, result))
}

#[tracing::instrument(
	name = "handle",
	level = "debug",
	parent = parent,
	skip_all,
)]
async fn execute(
	// we made a safety contract that Services will not go out of scope
	// during the request; this ensures a reference is accounted for at
	// the base frame of the task regardless of its detachment.
	_services: &Arc<Services>,
	req: http::Request<axum::body::Body>,
	next: axum::middleware::Next,
	parent: Span,
) -> Response {
	next.run(req).await
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

#[cold]
fn unhandled<Error: Debug>(e: Error) -> StatusCode {
	error!("unhandled error or panic during request: {e:?}");

	StatusCode::INTERNAL_SERVER_ERROR
}
