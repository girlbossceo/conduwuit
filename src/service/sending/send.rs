use std::{fmt::Debug, mem};

use http::{header::AUTHORIZATION, HeaderValue};
use ipaddress::IPAddress;
use reqwest::{Client, Method, Request, Response, Url};
use ruma::{
	api::{
		client::error::Error as RumaError, EndpointError, IncomingResponse, MatrixVersion, OutgoingRequest,
		SendAccessToken,
	},
	serde::Base64,
	server_util::authorization::XMatrix,
	ServerName,
};
use tracing::{debug, trace};

use super::{
	resolve,
	resolve::{ActualDest, CachedDest},
};
use crate::{debug_error, debug_warn, services, Error, Result};

#[tracing::instrument(skip_all, name = "send")]
pub async fn send<T>(client: &Client, dest: &ServerName, req: T) -> Result<T::IncomingResponse>
where
	T: OutgoingRequest + Debug + Send,
{
	if !services().globals.allow_federation() {
		return Err(Error::bad_config("Federation is disabled."));
	}

	let actual = resolve::get_actual_dest(dest).await?;
	let request = prepare::<T>(dest, &actual, req).await?;
	execute::<T>(client, dest, &actual, request).await
}

async fn execute<T>(
	client: &Client, dest: &ServerName, actual: &ActualDest, request: Request,
) -> Result<T::IncomingResponse>
where
	T: OutgoingRequest + Debug + Send,
{
	let method = request.method().clone();
	let url = request.url().clone();
	debug!(
		method = ?method,
		url = ?url,
		"Sending request",
	);
	match client.execute(request).await {
		Ok(response) => handle_response::<T>(dest, actual, &method, &url, response).await,
		Err(e) => handle_error::<T>(dest, actual, &method, &url, e),
	}
}

async fn prepare<T>(dest: &ServerName, actual: &ActualDest, req: T) -> Result<Request>
where
	T: OutgoingRequest + Debug + Send,
{
	const VERSIONS: [MatrixVersion; 1] = [MatrixVersion::V1_5];

	trace!("Preparing request");

	let mut http_request = req
		.try_into_http_request::<Vec<u8>>(&actual.string, SendAccessToken::IfRequired(""), &VERSIONS)
		.map_err(|_e| Error::BadServerResponse("Invalid destination"))?;

	sign_request::<T>(dest, &mut http_request);

	let request = Request::try_from(http_request)?;
	validate_url(request.url())?;

	Ok(request)
}

async fn handle_response<T>(
	dest: &ServerName, actual: &ActualDest, method: &Method, url: &Url, mut response: Response,
) -> Result<T::IncomingResponse>
where
	T: OutgoingRequest + Debug + Send,
{
	trace!("Received response from {} for {} with {}", actual.string, url, response.url());
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

	trace!("Waiting for response body");
	let body = response.bytes().await.unwrap_or_else(|e| {
		debug_error!("server error {}", e);
		Vec::new().into()
	}); // TODO: handle timeout

	let http_response = http_response_builder
		.body(body)
		.expect("reqwest body is valid http body");

	debug!("Got {status:?} for {method} {url}");
	if !status.is_success() {
		return Err(Error::Federation(dest.to_owned(), RumaError::from_http_response(http_response)));
	}

	let response = T::IncomingResponse::try_from_http_response(http_response);
	if response.is_ok() && !actual.cached {
		services().globals.resolver.set_cached_destination(
			dest.to_owned(),
			CachedDest {
				dest: actual.dest.clone(),
				host: actual.host.clone(),
				expire: CachedDest::default_expire(),
			},
		);
	}

	match response {
		Err(_e) => Err(Error::BadServerResponse("Server returned bad 200 response.")),
		Ok(response) => Ok(response),
	}
}

fn handle_error<T>(
	_dest: &ServerName, actual: &ActualDest, method: &Method, url: &Url, mut e: reqwest::Error,
) -> Result<T::IncomingResponse>
where
	T: OutgoingRequest + Debug + Send,
{
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

fn sign_request<T>(dest: &ServerName, http_request: &mut http::Request<Vec<u8>>)
where
	T: OutgoingRequest + Debug + Send,
{
	let mut req_map = serde_json::Map::new();
	if !http_request.body().is_empty() {
		req_map.insert(
			"content".to_owned(),
			serde_json::from_slice(http_request.body()).expect("body is valid json, we just created it"),
		);
	};

	req_map.insert("method".to_owned(), T::METADATA.method.to_string().into());
	req_map.insert(
		"uri".to_owned(),
		http_request
			.uri()
			.path_and_query()
			.expect("all requests have a path")
			.to_string()
			.into(),
	);
	req_map.insert("origin".to_owned(), services().globals.server_name().as_str().into());
	req_map.insert("destination".to_owned(), dest.as_str().into());

	let mut req_json = serde_json::from_value(req_map.into()).expect("valid JSON is valid BTreeMap");
	ruma::signatures::sign_json(
		services().globals.server_name().as_str(),
		services().globals.keypair(),
		&mut req_json,
	)
	.expect("our request json is what ruma expects");

	let req_json: serde_json::Map<String, serde_json::Value> =
		serde_json::from_slice(&serde_json::to_vec(&req_json).unwrap()).unwrap();

	let signatures = req_json["signatures"]
		.as_object()
		.expect("signatures object")
		.values()
		.map(|v| {
			v.as_object()
				.expect("server signatures object")
				.iter()
				.map(|(k, v)| (k, v.as_str().expect("server signature string")))
		});

	for signature_server in signatures {
		for s in signature_server {
			let key =
				s.0.as_str()
					.try_into()
					.expect("valid homeserver signing key ID");
			let sig = Base64::parse(s.1).expect("valid base64");

			http_request.headers_mut().insert(
				AUTHORIZATION,
				HeaderValue::from(&XMatrix::new(
					services().globals.config.server_name.clone(),
					dest.to_owned(),
					key,
					sig,
				)),
			);
		}
	}
}

fn validate_url(url: &Url) -> Result<()> {
	if let Some(url_host) = url.host_str() {
		if let Ok(ip) = IPAddress::parse(url_host) {
			trace!("Checking request URL IP {ip:?}");
			resolve::validate_ip(&ip)?;
		}
	}

	Ok(())
}
