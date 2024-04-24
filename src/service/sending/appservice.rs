use std::{fmt::Debug, mem};

use bytes::BytesMut;
use ruma::api::{appservice::Registration, IncomingResponse, MatrixVersion, OutgoingRequest, SendAccessToken};
use tracing::{trace, warn};

use crate::{debug_error, services, utils, Error, Result};

/// Sends a request to an appservice
///
/// Only returns Ok(None) if there is no url specified in the appservice
/// registration file
pub(crate) async fn send_request<T>(registration: Registration, request: T) -> Result<Option<T::IncomingResponse>>
where
	T: OutgoingRequest + Debug,
{
	let Some(dest) = registration.url else {
		return Ok(None);
	};

	trace!("Appservice URL \"{dest}\", Appservice ID: {}", registration.id);

	let hs_token = registration.hs_token.as_str();

	const VERSIONS: [MatrixVersion; 1] = [MatrixVersion::V1_0];

	let mut http_request = request
		.try_into_http_request::<BytesMut>(&dest, SendAccessToken::IfRequired(hs_token), &VERSIONS)
		.map_err(|e| {
			warn!("Failed to find destination {dest}: {e}");
			Error::BadServerResponse("Invalid appservice destination")
		})?
		.map(BytesMut::freeze);

	let mut parts = http_request.uri().clone().into_parts();
	let old_path_and_query = parts.path_and_query.unwrap().as_str().to_owned();
	let symbol = if old_path_and_query.contains('?') {
		"&"
	} else {
		"?"
	};

	parts.path_and_query = Some(
		(old_path_and_query + symbol + "access_token=" + hs_token)
			.parse()
			.unwrap(),
	);
	*http_request.uri_mut() = parts.try_into().expect("our manipulation is always valid");

	let reqwest_request = reqwest::Request::try_from(http_request)?;

	let mut response = services()
		.globals
		.client
		.appservice
		.execute(reqwest_request)
		.await
		.map_err(|e| {
			warn!("Could not send request to appservice \"{}\" at {dest}: {e}", registration.id);
			e
		})?;

	// reqwest::Response -> http::Response conversion
	let status = response.status();
	let mut http_response_builder = http::Response::builder()
		.status(status)
		.version(response.version());
	mem::swap(
		response.headers_mut(),
		http_response_builder
			.headers_mut()
			.expect("http::response::Builder is usable"),
	);

	let body = response.bytes().await?; // TODO: handle timeout

	if !status.is_success() {
		warn!(
			"Appservice \"{}\" returned unsuccessful HTTP response {status} at {dest}",
			registration.id
		);
		debug_error!("Appservice response bytes: {:?}", utils::string_from_bytes(&body));

		return Err(Error::BadServerResponse("Appservice returned unsuccessful HTTP response"));
	}

	let response = T::IncomingResponse::try_from_http_response(
		http_response_builder
			.body(body)
			.expect("reqwest body is valid http body"),
	);

	response.map(Some).map_err(|e| {
		warn!("Appservice \"{}\" returned invalid response bytes {dest}: {e}", registration.id);
		Error::BadServerResponse("Appservice returned bad/invalid response")
	})
}
