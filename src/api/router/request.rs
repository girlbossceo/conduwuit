use std::str;

use axum::{extract::Path, RequestExt, RequestPartsExt};
use bytes::Bytes;
use http::request::Parts;
use ruma::api::client::error::ErrorKind;
use serde::Deserialize;

use crate::{services, Error, Result};

#[derive(Deserialize)]
pub(super) struct QueryParams {
	pub(super) access_token: Option<String>,
	pub(super) user_id: Option<String>,
}

pub(super) struct Request {
	pub(super) path: Path<Vec<String>>,
	pub(super) query: QueryParams,
	pub(super) body: Bytes,
	pub(super) parts: Parts,
}

pub(super) async fn from(request: hyper::Request<axum::body::Body>) -> Result<Request> {
	let limited = request.with_limited_body();
	let (mut parts, body) = limited.into_parts();

	let path: Path<Vec<String>> = parts.extract().await?;
	let query = serde_html_form::from_str(parts.uri.query().unwrap_or_default())
		.map_err(|_| Error::BadRequest(ErrorKind::Unknown, "Failed to read query parameters"))?;

	let max_body_size = services()
		.globals
		.config
		.max_request_size
		.try_into()
		.expect("failed to convert max request size");

	let body = axum::body::to_bytes(body, max_body_size)
		.await
		.map_err(|_| Error::BadRequest(ErrorKind::TooLarge, "Request body too large"))?;

	Ok(Request {
		path,
		query,
		body,
		parts,
	})
}
