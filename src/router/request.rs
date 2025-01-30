use std::sync::{atomic::Ordering, Arc};

use axum::{
	extract::State,
	response::{IntoResponse, Response},
};
use conduwuit::{debug, debug_error, debug_warn, err, error, trace, Result};
use conduwuit_service::Services;
use http::{Method, StatusCode, Uri};

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
	#[cfg(debug_assertions)]
	conduwuit::defer! {{
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

	let method = req.method().clone();
	let uri = req.uri().clone();
	services
		.server
		.runtime()
		.spawn(next.run(req))
		.await
		.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
		.and_then(|result| handle_result(&method, &uri, result))
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
