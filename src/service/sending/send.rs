use std::{fmt::Debug, mem};

use conduit::{
	debug, debug_error, debug_warn, err, error::inspect_debug_log, implement, trace, utils::string::EMPTY, Err, Error,
	Result,
};
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

use crate::{
	resolver,
	resolver::{actual::ActualDest, cache::CachedDest},
};

impl super::Service {
	#[tracing::instrument(skip(self, client, req), name = "send")]
	pub async fn send<T>(&self, client: &Client, dest: &ServerName, req: T) -> Result<T::IncomingResponse>
	where
		T: OutgoingRequest + Debug + Send,
	{
		if !self.server.config.allow_federation {
			return Err!(Config("allow_federation", "Federation is disabled."));
		}

		if self
			.server
			.config
			.forbidden_remote_server_names
			.contains(dest)
		{
			return Err!(Request(Forbidden(debug_warn!("Federation with this {dest} is not allowed."))));
		}

		let actual = self.services.resolver.get_actual_dest(dest).await?;
		let request = self.prepare::<T>(dest, &actual, req).await?;
		self.execute::<T>(dest, &actual, request, client).await
	}

	async fn execute<T>(
		&self, dest: &ServerName, actual: &ActualDest, request: Request, client: &Client,
	) -> Result<T::IncomingResponse>
	where
		T: OutgoingRequest + Debug + Send,
	{
		let url = request.url().clone();
		let method = request.method().clone();

		debug!(?method, ?url, "Sending request");
		match client.execute(request).await {
			Ok(response) => handle_response::<T>(&self.services.resolver, dest, actual, &method, &url, response).await,
			Err(error) => handle_error::<T>(dest, actual, &method, &url, error),
		}
	}

	async fn prepare<T>(&self, dest: &ServerName, actual: &ActualDest, req: T) -> Result<Request>
	where
		T: OutgoingRequest + Debug + Send,
	{
		const VERSIONS: [MatrixVersion; 1] = [MatrixVersion::V1_11];
		const SATIR: SendAccessToken<'_> = SendAccessToken::IfRequired(EMPTY);

		trace!("Preparing request");
		let mut http_request = req
			.try_into_http_request::<Vec<u8>>(&actual.string, SATIR, &VERSIONS)
			.map_err(|e| err!(BadServerResponse("Invalid destination: {e:?}")))?;

		self.sign_request::<T>(dest, &mut http_request);

		let request = Request::try_from(http_request)?;
		self.validate_url(request.url())?;

		Ok(request)
	}

	fn validate_url(&self, url: &Url) -> Result<()> {
		if let Some(url_host) = url.host_str() {
			if let Ok(ip) = IPAddress::parse(url_host) {
				trace!("Checking request URL IP {ip:?}");
				self.services.resolver.validate_ip(&ip)?;
			}
		}

		Ok(())
	}
}

async fn handle_response<T>(
	resolver: &resolver::Service, dest: &ServerName, actual: &ActualDest, method: &Method, url: &Url,
	mut response: Response,
) -> Result<T::IncomingResponse>
where
	T: OutgoingRequest + Debug + Send,
{
	let status = response.status();
	trace!(
		?status, ?method,
		request_url = ?url,
		response_url = ?response.url(),
		"Received response from {}",
		actual.string,
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
		return Err(Error::Federation(dest.to_owned(), RumaError::from_http_response(http_response)));
	}

	let response = T::IncomingResponse::try_from_http_response(http_response);
	if response.is_ok() && !actual.cached {
		resolver.set_cached_destination(
			dest.to_owned(),
			CachedDest {
				dest: actual.dest.clone(),
				host: actual.host.clone(),
				expire: CachedDest::default_expire(),
			},
		);
	}

	response.map_err(|e| err!(BadServerResponse("Server returned bad 200 response: {e:?}")))
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

#[implement(super::Service)]
fn sign_request<T>(&self, dest: &ServerName, http_request: &mut http::Request<Vec<u8>>)
where
	T: OutgoingRequest + Debug + Send,
{
	let mut req_map = serde_json::Map::with_capacity(8);
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
	req_map.insert("origin".to_owned(), self.services.globals.server_name().to_string().into());
	req_map.insert("destination".to_owned(), dest.as_str().into());

	let mut req_json = serde_json::from_value(req_map.into()).expect("valid JSON is valid BTreeMap");
	self.services
		.server_keys
		.sign_json(&mut req_json)
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
					self.services.globals.server_name().to_owned(),
					dest.to_owned(),
					key,
					sig,
				)),
			);
		}
	}
}
