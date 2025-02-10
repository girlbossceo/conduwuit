use std::{fmt::Debug, mem};

use bytes::Bytes;
use conduwuit::{
	debug, debug::INFO_SPAN_LEVEL, debug_error, debug_warn, err, error::inspect_debug_log,
	implement, trace, utils::string::EMPTY, Err, Error, Result,
};
use http::{header::AUTHORIZATION, HeaderValue};
use ipaddress::IPAddress;
use reqwest::{Client, Method, Request, Response, Url};
use ruma::{
	api::{
		client::error::Error as RumaError, EndpointError, IncomingResponse, MatrixVersion,
		OutgoingRequest, SendAccessToken,
	},
	serde::Base64,
	server_util::authorization::XMatrix,
	CanonicalJsonObject, CanonicalJsonValue, ServerName, ServerSigningKeyId,
};

use crate::resolver::actual::ActualDest;

/// Sends a request to a federation server
#[implement(super::Service)]
#[tracing::instrument(skip_all, name = "request", level = "debug")]
pub async fn execute<T>(&self, dest: &ServerName, request: T) -> Result<T::IncomingResponse>
where
	T: OutgoingRequest + Debug + Send,
{
	let client = &self.services.client.federation;
	self.execute_on(client, dest, request).await
}

/// Like execute() but with a very large timeout
#[implement(super::Service)]
#[tracing::instrument(skip_all, name = "synapse", level = "debug")]
pub async fn execute_synapse<T>(
	&self,
	dest: &ServerName,
	request: T,
) -> Result<T::IncomingResponse>
where
	T: OutgoingRequest + Debug + Send,
{
	let client = &self.services.client.synapse;
	self.execute_on(client, dest, request).await
}

#[implement(super::Service)]
#[tracing::instrument(
		name = "fed",
		level = INFO_SPAN_LEVEL,
		skip(self, client, request),
	)]
pub async fn execute_on<T>(
	&self,
	client: &Client,
	dest: &ServerName,
	request: T,
) -> Result<T::IncomingResponse>
where
	T: OutgoingRequest + Send,
{
	if !self.services.server.config.allow_federation {
		return Err!(Config("allow_federation", "Federation is disabled."));
	}

	if self.services.moderation.is_remote_server_forbidden(dest) {
		return Err!(Request(Forbidden(debug_warn!("Federation with {dest} is not allowed."))));
	}

	let actual = self.services.resolver.get_actual_dest(dest).await?;
	let request = into_http_request::<T>(&actual, request)?;
	let request = self.prepare(dest, request)?;
	self.perform::<T>(dest, &actual, request, client).await
}

#[implement(super::Service)]
async fn perform<T>(
	&self,
	dest: &ServerName,
	actual: &ActualDest,
	request: Request,
	client: &Client,
) -> Result<T::IncomingResponse>
where
	T: OutgoingRequest + Send,
{
	let url = request.url().clone();
	let method = request.method().clone();

	debug!(?method, ?url, "Sending request");
	match client.execute(request).await {
		| Ok(response) => handle_response::<T>(dest, actual, &method, &url, response).await,
		| Err(error) =>
			Err(handle_error(actual, &method, &url, error).expect_err("always returns error")),
	}
}

#[implement(super::Service)]
fn prepare(&self, dest: &ServerName, mut request: http::Request<Vec<u8>>) -> Result<Request> {
	self.sign_request(&mut request, dest);

	let request = Request::try_from(request)?;
	self.validate_url(request.url())?;
	self.services.server.check_running()?;

	Ok(request)
}

#[implement(super::Service)]
fn validate_url(&self, url: &Url) -> Result<()> {
	if let Some(url_host) = url.host_str() {
		if let Ok(ip) = IPAddress::parse(url_host) {
			trace!("Checking request URL IP {ip:?}");
			self.services.resolver.validate_ip(&ip)?;
		}
	}

	Ok(())
}

async fn handle_response<T>(
	dest: &ServerName,
	actual: &ActualDest,
	method: &Method,
	url: &Url,
	response: Response,
) -> Result<T::IncomingResponse>
where
	T: OutgoingRequest + Send,
{
	let response = into_http_response(dest, actual, method, url, response).await?;

	T::IncomingResponse::try_from_http_response(response)
		.map_err(|e| err!(BadServerResponse("Server returned bad 200 response: {e:?}")))
}

async fn into_http_response(
	dest: &ServerName,
	actual: &ActualDest,
	method: &Method,
	url: &Url,
	mut response: Response,
) -> Result<http::Response<Bytes>> {
	let status = response.status();
	trace!(
		?status, ?method,
		request_url = ?url,
		response_url = ?response.url(),
		"Received response from {}",
		actual.string(),
	);

	let mut http_response_builder = http::Response::builder()
		.status(status)
		.version(response.version());

	mem::swap(
		response.headers_mut(),
		http_response_builder
			.headers_mut()
			.expect("http::response::Builder is usable"),
	);

	// TODO: handle timeout
	trace!("Waiting for response body...");
	let body = response
		.bytes()
		.await
		.inspect_err(inspect_debug_log)
		.unwrap_or_else(|_| Vec::new().into());

	let http_response = http_response_builder
		.body(body)
		.expect("reqwest body is valid http body");

	debug!("Got {status:?} for {method} {url}");
	if !status.is_success() {
		return Err(Error::Federation(
			dest.to_owned(),
			RumaError::from_http_response(http_response),
		));
	}

	Ok(http_response)
}

fn handle_error(
	actual: &ActualDest,
	method: &Method,
	url: &Url,
	mut e: reqwest::Error,
) -> Result {
	if e.is_timeout() || e.is_connect() {
		e = e.without_url();
		debug_warn!("{e:?}");
	} else if e.is_redirect() {
		debug_error!(
			method = ?method,
			url = ?url,
			final_url = ?e.url(),
			"Redirect loop {}: {}",
			actual.host,
			e,
		);
	} else {
		debug_error!("{e:?}");
	}

	Err(e.into())
}

#[implement(super::Service)]
fn sign_request(&self, http_request: &mut http::Request<Vec<u8>>, dest: &ServerName) {
	type Member = (String, Value);
	type Value = CanonicalJsonValue;
	type Object = CanonicalJsonObject;

	let origin = &self.services.server.name;
	let body = http_request.body();
	let uri = http_request
		.uri()
		.path_and_query()
		.expect("http::Request missing path_and_query");

	let mut req: Object = if !body.is_empty() {
		let content: CanonicalJsonValue =
			serde_json::from_slice(body).expect("failed to serialize body");

		let authorization: [Member; 5] = [
			("content".into(), content),
			("destination".into(), dest.as_str().into()),
			("method".into(), http_request.method().as_str().into()),
			("origin".into(), origin.as_str().into()),
			("uri".into(), uri.to_string().into()),
		];

		authorization.into()
	} else {
		let authorization: [Member; 4] = [
			("destination".into(), dest.as_str().into()),
			("method".into(), http_request.method().as_str().into()),
			("origin".into(), origin.as_str().into()),
			("uri".into(), uri.to_string().into()),
		];

		authorization.into()
	};

	self.services
		.server_keys
		.sign_json(&mut req)
		.expect("request signing failed");

	let signatures = req["signatures"]
		.as_object()
		.and_then(|object| object[origin.as_str()].as_object())
		.expect("origin signatures object");

	let key: &ServerSigningKeyId = signatures
		.keys()
		.next()
		.map(|k| k.as_str().try_into())
		.expect("at least one signature from this origin")
		.expect("keyid is json string");

	let sig: Base64 = signatures
		.values()
		.next()
		.map(|s| s.as_str().map(Base64::parse))
		.expect("at least one signature from this origin")
		.expect("signature is json string")
		.expect("signature is valid base64");

	let x_matrix = XMatrix::new(origin.into(), dest.into(), key.into(), sig);
	let authorization = HeaderValue::from(&x_matrix);
	let authorization = http_request
		.headers_mut()
		.insert(AUTHORIZATION, authorization);

	debug_assert!(authorization.is_none(), "Authorization header already present");
}

fn into_http_request<T>(actual: &ActualDest, request: T) -> Result<http::Request<Vec<u8>>>
where
	T: OutgoingRequest + Send,
{
	const VERSIONS: [MatrixVersion; 1] = [MatrixVersion::V1_11];
	const SATIR: SendAccessToken<'_> = SendAccessToken::IfRequired(EMPTY);

	let http_request = request
		.try_into_http_request::<Vec<u8>>(actual.string().as_str(), SATIR, &VERSIONS)
		.map_err(|e| err!(BadServerResponse("Invalid destination: {e:?}")))?;

	Ok(http_request)
}
