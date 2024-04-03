use std::{
	fmt::Debug,
	mem,
	net::{IpAddr, SocketAddr},
};

use futures_util::TryFutureExt;
use hickory_resolver::{error::ResolveError, lookup::SrvLookup};
use http::{header::AUTHORIZATION, HeaderValue};
use ipaddress::IPAddress;
use ruma::{
	api::{
		client::error::Error as RumaError, EndpointError, IncomingResponse, MatrixVersion, OutgoingRequest,
		SendAccessToken,
	},
	OwnedServerName, ServerName,
};
use tracing::{debug, info, warn};

use crate::{services, Error, Result};

/// Wraps either an literal IP address plus port, or a hostname plus complement
/// (colon-plus-port if it was specified).
///
/// Note: A `FedDest::Named` might contain an IP address in string form if there
/// was no port specified to construct a `SocketAddr` with.
///
/// # Examples:
/// ```rust
/// # use conduit::api::server_server::FedDest;
/// # fn main() -> Result<(), std::net::AddrParseError> {
/// FedDest::Literal("198.51.100.3:8448".parse()?);
/// FedDest::Literal("[2001:db8::4:5]:443".parse()?);
/// FedDest::Named("matrix.example.org".to_owned(), String::new());
/// FedDest::Named("matrix.example.org".to_owned(), ":8448".to_owned());
/// FedDest::Named("198.51.100.5".to_owned(), String::new());
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FedDest {
	Literal(SocketAddr),
	Named(String, String),
}

#[tracing::instrument(skip_all, name = "send")]
pub(crate) async fn send_request<T>(destination: &ServerName, request: T) -> Result<T::IncomingResponse>
where
	T: OutgoingRequest + Debug,
{
	if !services().globals.allow_federation() {
		return Err(Error::bad_config("Federation is disabled."));
	}

	if destination == services().globals.server_name() {
		return Err(Error::bad_config("Won't send federation request to ourselves"));
	}

	if destination.is_ip_literal() || IPAddress::is_valid(destination.host()) {
		info!(
			"Destination {} is an IP literal, checking against IP range denylist.",
			destination
		);
		let ip = IPAddress::parse(destination.host()).map_err(|e| {
			warn!("Failed to parse IP literal from string: {}", e);
			Error::BadServerResponse("Invalid IP address")
		})?;

		let cidr_ranges_s = services().globals.ip_range_denylist().to_vec();
		let mut cidr_ranges: Vec<IPAddress> = Vec::new();

		for cidr in cidr_ranges_s {
			cidr_ranges.push(IPAddress::parse(cidr).expect("we checked this at startup"));
		}

		debug!("List of pushed CIDR ranges: {:?}", cidr_ranges);

		for cidr in cidr_ranges {
			if cidr.includes(&ip) {
				return Err(Error::BadServerResponse("Not allowed to send requests to this IP"));
			}
		}

		info!("IP literal {} is allowed.", destination);
	}

	debug!("Preparing to send request to {destination}");

	let mut write_destination_to_cache = false;

	let cached_result = services()
		.globals
		.actual_destinations()
		.read()
		.await
		.get(destination)
		.cloned();

	let (actual_destination, host) = if let Some(result) = cached_result {
		result
	} else {
		write_destination_to_cache = true;

		let result = resolve_actual_destination(destination).await;

		(result.0, result.1.into_uri_string())
	};

	let actual_destination_str = actual_destination.clone().into_https_string();

	let mut http_request = request
		.try_into_http_request::<Vec<u8>>(
			&actual_destination_str,
			SendAccessToken::IfRequired(""),
			&[MatrixVersion::V1_5],
		)
		.map_err(|e| {
			warn!("Failed to find destination {}: {}", actual_destination_str, e);
			Error::BadServerResponse("Invalid destination")
		})?;

	let mut request_map = serde_json::Map::new();

	if !http_request.body().is_empty() {
		request_map.insert(
			"content".to_owned(),
			serde_json::from_slice(http_request.body()).expect("body is valid json, we just created it"),
		);
	};

	request_map.insert("method".to_owned(), T::METADATA.method.to_string().into());
	request_map.insert(
		"uri".to_owned(),
		http_request
			.uri()
			.path_and_query()
			.expect("all requests have a path")
			.to_string()
			.into(),
	);
	request_map.insert("origin".to_owned(), services().globals.server_name().as_str().into());
	request_map.insert("destination".to_owned(), destination.as_str().into());

	let mut request_json = serde_json::from_value(request_map.into()).expect("valid JSON is valid BTreeMap");

	ruma::signatures::sign_json(
		services().globals.server_name().as_str(),
		services().globals.keypair(),
		&mut request_json,
	)
	.expect("our request json is what ruma expects");

	let request_json: serde_json::Map<String, serde_json::Value> =
		serde_json::from_slice(&serde_json::to_vec(&request_json).unwrap()).unwrap();

	let signatures = request_json["signatures"]
		.as_object()
		.unwrap()
		.values()
		.map(|v| {
			v.as_object()
				.unwrap()
				.iter()
				.map(|(k, v)| (k, v.as_str().unwrap()))
		});

	for signature_server in signatures {
		for s in signature_server {
			http_request.headers_mut().insert(
				AUTHORIZATION,
				HeaderValue::from_str(&format!(
					"X-Matrix origin={},key=\"{}\",sig=\"{}\"",
					services().globals.server_name(),
					s.0,
					s.1
				))
				.unwrap(),
			);
		}
	}

	let reqwest_request = reqwest::Request::try_from(http_request)?;

	let url = reqwest_request.url().clone();

	if let Some(url_host) = url.host_str() {
		debug!("Checking request URL for IP");
		if let Ok(ip) = IPAddress::parse(url_host) {
			let cidr_ranges_s = services().globals.ip_range_denylist().to_vec();
			let mut cidr_ranges: Vec<IPAddress> = Vec::new();

			for cidr in cidr_ranges_s {
				cidr_ranges.push(IPAddress::parse(cidr).expect("we checked this at startup"));
			}

			for cidr in cidr_ranges {
				if cidr.includes(&ip) {
					return Err(Error::BadServerResponse("Not allowed to send requests to this IP"));
				}
			}
		}
	}

	debug!("Sending request to {destination} at {url}");
	let response = services()
		.globals
		.client
		.federation
		.execute(reqwest_request)
		.await;
	debug!("Received response from {destination} at {url}");

	match response {
		Ok(mut response) => {
			// reqwest::Response -> http::Response conversion

			debug!("Checking response destination's IP");
			if let Some(remote_addr) = response.remote_addr() {
				if let Ok(ip) = IPAddress::parse(remote_addr.ip().to_string()) {
					let cidr_ranges_s = services().globals.ip_range_denylist().to_vec();
					let mut cidr_ranges: Vec<IPAddress> = Vec::new();

					for cidr in cidr_ranges_s {
						cidr_ranges.push(IPAddress::parse(cidr).expect("we checked this at startup"));
					}

					for cidr in cidr_ranges {
						if cidr.includes(&ip) {
							return Err(Error::BadServerResponse("Not allowed to send requests to this IP"));
						}
					}
				}
			}

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

			debug!("Getting response bytes from {destination}");
			let body = response.bytes().await.unwrap_or_else(|e| {
				info!("server error {}", e);
				Vec::new().into()
			}); // TODO: handle timeout
			debug!("Got response bytes from {destination}");

			if !status.is_success() {
				debug!(
					"Response not successful\n{} {}: {}",
					url,
					status,
					String::from_utf8_lossy(&body)
						.lines()
						.collect::<Vec<_>>()
						.join(" ")
				);
			}

			let http_response = http_response_builder
				.body(body)
				.expect("reqwest body is valid http body");

			if status.is_success() {
				debug!("Parsing response bytes from {destination}");
				let response = T::IncomingResponse::try_from_http_response(http_response);
				if response.is_ok() && write_destination_to_cache {
					services()
						.globals
						.actual_destinations()
						.write()
						.await
						.insert(OwnedServerName::from(destination), (actual_destination, host));
				}

				response.map_err(|e| {
					info!("Invalid 200 response from {} on: {} {}", &destination, url, e);
					Error::BadServerResponse("Server returned bad 200 response.")
				})
			} else {
				debug!("Returning error from {destination}");
				Err(Error::FederationError(
					destination.to_owned(),
					RumaError::from_http_response(http_response),
				))
			}
		},
		Err(e) => {
			// we do not need to log that servers in a room are dead, this is normal in
			// public rooms and just spams the logs.
			if e.is_timeout() {
				debug!(
					"Timed out sending request to {} at {}: {}",
					destination, actual_destination_str, e
				);
			} else if e.is_connect() {
				debug!("Failed to connect to {} at {}: {}", destination, actual_destination_str, e);
			} else if e.is_redirect() {
				debug!(
					"Redirect loop sending request to {} at {}: {}\nFinal URL: {:?}",
					destination,
					actual_destination_str,
					e,
					e.url()
				);
			} else {
				info!("Could not send request to {} at {}: {}", destination, actual_destination_str, e);
			}

			Err(e.into())
		},
	}
}

fn get_ip_with_port(destination_str: &str) -> Option<FedDest> {
	if let Ok(destination) = destination_str.parse::<SocketAddr>() {
		Some(FedDest::Literal(destination))
	} else if let Ok(ip_addr) = destination_str.parse::<IpAddr>() {
		Some(FedDest::Literal(SocketAddr::new(ip_addr, 8448)))
	} else {
		None
	}
}

fn add_port_to_hostname(destination_str: &str) -> FedDest {
	let (host, port) = match destination_str.find(':') {
		None => (destination_str, ":8448"),
		Some(pos) => destination_str.split_at(pos),
	};
	FedDest::Named(host.to_owned(), port.to_owned())
}

/// Returns: `actual_destination`, host header
/// Implemented according to the specification at <https://matrix.org/docs/spec/server_server/r0.1.4#resolving-server-names>
/// Numbers in comments below refer to bullet points in linked section of
/// specification
#[tracing::instrument(skip_all, name = "resolve")]
async fn resolve_actual_destination(destination: &'_ ServerName) -> (FedDest, FedDest) {
	debug!("Finding actual destination for {destination}");
	let destination_str = destination.as_str().to_owned();
	let mut hostname = destination_str.clone();
	let actual_destination = match get_ip_with_port(&destination_str) {
		Some(host_port) => {
			debug!("1: IP literal with provided or default port");
			host_port
		},
		None => {
			if let Some(pos) = destination_str.find(':') {
				debug!("2: Hostname with included port");

				let (host, port) = destination_str.split_at(pos);
				query_and_cache_override(host, host, port.parse::<u16>().unwrap_or(8448)).await;

				FedDest::Named(host.to_owned(), port.to_owned())
			} else {
				debug!("Requesting well known for {destination}");
				if let Some(delegated_hostname) = request_well_known(destination.as_str()).await {
					debug!("3: A .well-known file is available");
					hostname = add_port_to_hostname(&delegated_hostname).into_uri_string();
					match get_ip_with_port(&delegated_hostname) {
						Some(host_and_port) => host_and_port, // 3.1: IP literal in .well-known file
						None => {
							if let Some(pos) = delegated_hostname.find(':') {
								debug!("3.2: Hostname with port in .well-known file");

								let (host, port) = delegated_hostname.split_at(pos);
								query_and_cache_override(host, host, port.parse::<u16>().unwrap_or(8448)).await;

								FedDest::Named(host.to_owned(), port.to_owned())
							} else {
								debug!("Delegated hostname has no port in this branch");
								if let Some(hostname_override) = query_srv_record(&delegated_hostname).await {
									debug!("3.3: SRV lookup successful");

									let force_port = hostname_override.port();
									query_and_cache_override(
										&delegated_hostname,
										&hostname_override.hostname(),
										force_port.unwrap_or(8448),
									)
									.await;

									if let Some(port) = force_port {
										FedDest::Named(delegated_hostname, format!(":{port}"))
									} else {
										add_port_to_hostname(&delegated_hostname)
									}
								} else {
									debug!("3.4: No SRV records, just use the hostname from .well-known");
									query_and_cache_override(&delegated_hostname, &delegated_hostname, 8448).await;
									add_port_to_hostname(&delegated_hostname)
								}
							}
						},
					}
				} else {
					debug!("4: No .well-known or an error occured");
					if let Some(hostname_override) = query_srv_record(&destination_str).await {
						debug!("4: SRV record found");

						let force_port = hostname_override.port();
						query_and_cache_override(&hostname, &hostname_override.hostname(), force_port.unwrap_or(8448))
							.await;

						if let Some(port) = force_port {
							FedDest::Named(hostname.clone(), format!(":{port}"))
						} else {
							add_port_to_hostname(&hostname)
						}
					} else {
						debug!("5: No SRV record found");
						query_and_cache_override(&destination_str, &destination_str, 8448).await;
						add_port_to_hostname(&destination_str)
					}
				}
			}
		},
	};

	// Can't use get_ip_with_port here because we don't want to add a port
	// to an IP address if it wasn't specified
	let hostname = if let Ok(addr) = hostname.parse::<SocketAddr>() {
		FedDest::Literal(addr)
	} else if let Ok(addr) = hostname.parse::<IpAddr>() {
		FedDest::Named(addr.to_string(), ":8448".to_owned())
	} else if let Some(pos) = hostname.find(':') {
		let (host, port) = hostname.split_at(pos);
		FedDest::Named(host.to_owned(), port.to_owned())
	} else {
		FedDest::Named(hostname, ":8448".to_owned())
	};

	debug!("Actual destination: {actual_destination:?} hostname: {hostname:?}");
	(actual_destination, hostname)
}

async fn query_and_cache_override(overname: &'_ str, hostname: &'_ str, port: u16) {
	match services()
		.globals
		.dns_resolver()
		.lookup_ip(hostname.to_owned())
		.await
	{
		Ok(override_ip) => {
			debug!("Caching result of {:?} overriding {:?}", hostname, overname);

			services()
				.globals
				.resolver
				.overrides
				.write()
				.unwrap()
				.insert(overname.to_owned(), (override_ip.iter().collect(), port));
		},
		Err(e) => {
			debug!("Got {:?} for {:?} to override {:?}", e.kind(), hostname, overname);
		},
	}
}

async fn query_srv_record(hostname: &'_ str) -> Option<FedDest> {
	fn handle_successful_srv(srv: &SrvLookup) -> Option<FedDest> {
		srv.iter().next().map(|result| {
			FedDest::Named(
				result.target().to_string().trim_end_matches('.').to_owned(),
				format!(":{}", result.port()),
			)
		})
	}

	async fn lookup_srv(hostname: &str) -> Result<SrvLookup, ResolveError> {
		debug!("querying SRV for {:?}", hostname);
		let hostname = hostname.trim_end_matches('.');
		services()
			.globals
			.dns_resolver()
			.srv_lookup(hostname.to_owned())
			.await
	}

	let first_hostname = format!("_matrix-fed._tcp.{hostname}.");
	let second_hostname = format!("_matrix._tcp.{hostname}.");

	lookup_srv(&first_hostname)
		.or_else(|_| {
			debug!("Querying deprecated _matrix SRV record for host {:?}", hostname);
			lookup_srv(&second_hostname)
		})
		.and_then(|srv_lookup| async move { Ok(handle_successful_srv(&srv_lookup)) })
		.await
		.ok()
		.flatten()
}

async fn request_well_known(destination: &str) -> Option<String> {
	if !services()
		.globals
		.resolver
		.overrides
		.read()
		.unwrap()
		.contains_key(destination)
	{
		query_and_cache_override(destination, destination, 8448).await;
	}

	let response = services()
		.globals
		.client
		.well_known
		.get(&format!("https://{destination}/.well-known/matrix/server"))
		.send()
		.await;
	debug!("Got well known response");
	debug!("Well known response: {:?}", response);

	if let Err(e) = &response {
		debug!("Well known error: {e:?}");
		return None;
	}

	let text = response.ok()?.text().await;

	debug!("Got well known response text");
	debug!("Well known response text: {:?}", text);

	if text.as_ref().ok()?.len() > 10000 {
		debug!(
			"Well known response for destination '{destination}' exceeded past 10000 characters, assuming no \
			 well-known."
		);
		return None;
	}

	let body: serde_json::Value = serde_json::from_str(&text.ok()?).ok()?;
	debug!("serde_json body of well known text: {}", body);

	Some(body.get("m.server")?.as_str()?.to_owned())
}

impl FedDest {
	fn into_https_string(self) -> String {
		match self {
			Self::Literal(addr) => format!("https://{addr}"),
			Self::Named(host, port) => format!("https://{host}{port}"),
		}
	}

	fn into_uri_string(self) -> String {
		match self {
			Self::Literal(addr) => addr.to_string(),
			Self::Named(host, port) => host + &port,
		}
	}

	fn hostname(&self) -> String {
		match &self {
			Self::Literal(addr) => addr.ip().to_string(),
			Self::Named(host, _) => host.clone(),
		}
	}

	fn port(&self) -> Option<u16> {
		match &self {
			Self::Literal(addr) => Some(addr.port()),
			Self::Named(_, port) => port[1..].parse().ok(),
		}
	}
}

#[cfg(test)]
mod tests {
	use super::{add_port_to_hostname, get_ip_with_port, FedDest};

	#[test]
	fn ips_get_default_ports() {
		assert_eq!(
			get_ip_with_port("1.1.1.1"),
			Some(FedDest::Literal("1.1.1.1:8448".parse().unwrap()))
		);
		assert_eq!(
			get_ip_with_port("dead:beef::"),
			Some(FedDest::Literal("[dead:beef::]:8448".parse().unwrap()))
		);
	}

	#[test]
	fn ips_keep_custom_ports() {
		assert_eq!(
			get_ip_with_port("1.1.1.1:1234"),
			Some(FedDest::Literal("1.1.1.1:1234".parse().unwrap()))
		);
		assert_eq!(
			get_ip_with_port("[dead::beef]:8933"),
			Some(FedDest::Literal("[dead::beef]:8933".parse().unwrap()))
		);
	}

	#[test]
	fn hostnames_get_default_ports() {
		assert_eq!(
			add_port_to_hostname("example.com"),
			FedDest::Named(String::from("example.com"), String::from(":8448"))
		);
	}

	#[test]
	fn hostnames_keep_custom_ports() {
		assert_eq!(
			add_port_to_hostname("example.com:1337"),
			FedDest::Named(String::from("example.com"), String::from(":1337"))
		);
	}
}
