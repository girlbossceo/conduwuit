use std::{
	fmt,
	fmt::Debug,
	net::{IpAddr, SocketAddr},
	time::SystemTime,
};

use hickory_resolver::{error::ResolveError, lookup::SrvLookup};
use ipaddress::IPAddress;
use ruma::{OwnedServerName, ServerName};
use tracing::{debug, error, trace};

use crate::{debug_error, debug_info, debug_warn, services, utils::rand, Error, Result};

/// Wraps either an literal IP address plus port, or a hostname plus complement
/// (colon-plus-port if it was specified).
///
/// Note: A `FedDest::Named` might contain an IP address in string form if there
/// was no port specified to construct a `SocketAddr` with.
///
/// # Examples:
/// ```rust
/// # use conduit_service::sending::FedDest;
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

#[derive(Clone, Debug)]
pub(crate) struct ActualDest {
	pub(crate) dest: FedDest,
	pub(crate) host: String,
	pub(crate) string: String,
	pub(crate) cached: bool,
}

#[derive(Clone, Debug)]
pub struct CachedDest {
	pub dest: FedDest,
	pub host: String,
	pub expire: SystemTime,
}

#[derive(Clone, Debug)]
pub struct CachedOverride {
	pub ips: Vec<IpAddr>,
	pub port: u16,
	pub expire: SystemTime,
}

#[tracing::instrument(skip_all, name = "resolve")]
pub(crate) async fn get_actual_dest(server_name: &ServerName) -> Result<ActualDest> {
	let cached;
	let cached_result = services()
		.globals
		.resolver
		.get_cached_destination(server_name);

	let CachedDest {
		dest,
		host,
		..
	} = if let Some(result) = cached_result {
		cached = true;
		result
	} else {
		cached = false;
		validate_dest(server_name)?;
		resolve_actual_dest(server_name, true).await?
	};

	let string = dest.clone().into_https_string();
	Ok(ActualDest {
		dest,
		host,
		string,
		cached,
	})
}

/// Returns: `actual_destination`, host header
/// Implemented according to the specification at <https://matrix.org/docs/spec/server_server/r0.1.4#resolving-server-names>
/// Numbers in comments below refer to bullet points in linked section of
/// specification
#[tracing::instrument(skip_all, name = "actual")]
pub async fn resolve_actual_dest(dest: &ServerName, cache: bool) -> Result<CachedDest> {
	trace!("Finding actual destination for {dest}");
	let mut host = dest.as_str().to_owned();
	let actual_dest = match get_ip_with_port(dest.as_str()) {
		Some(host_port) => actual_dest_1(host_port)?,
		None => {
			if let Some(pos) = dest.as_str().find(':') {
				actual_dest_2(dest, cache, pos).await?
			} else if let Some(delegated) = request_well_known(dest.as_str()).await? {
				actual_dest_3(&mut host, cache, delegated).await?
			} else if let Some(overrider) = query_srv_record(dest.as_str()).await? {
				actual_dest_4(&host, cache, overrider).await?
			} else {
				actual_dest_5(dest, cache).await?
			}
		},
	};

	// Can't use get_ip_with_port here because we don't want to add a port
	// to an IP address if it wasn't specified
	let host = if let Ok(addr) = host.parse::<SocketAddr>() {
		FedDest::Literal(addr)
	} else if let Ok(addr) = host.parse::<IpAddr>() {
		FedDest::Named(addr.to_string(), ":8448".to_owned())
	} else if let Some(pos) = host.find(':') {
		let (host, port) = host.split_at(pos);
		FedDest::Named(host.to_owned(), port.to_owned())
	} else {
		FedDest::Named(host, ":8448".to_owned())
	};

	debug!("Actual destination: {actual_dest:?} hostname: {host:?}");
	Ok(CachedDest {
		dest: actual_dest,
		host: host.into_uri_string(),
		expire: CachedDest::default_expire(),
	})
}

fn actual_dest_1(host_port: FedDest) -> Result<FedDest> {
	debug!("1: IP literal with provided or default port");
	Ok(host_port)
}

async fn actual_dest_2(dest: &ServerName, cache: bool, pos: usize) -> Result<FedDest> {
	debug!("2: Hostname with included port");
	let (host, port) = dest.as_str().split_at(pos);
	conditional_query_and_cache_override(host, host, port.parse::<u16>().unwrap_or(8448), cache).await?;
	Ok(FedDest::Named(host.to_owned(), port.to_owned()))
}

async fn actual_dest_3(host: &mut String, cache: bool, delegated: String) -> Result<FedDest> {
	debug!("3: A .well-known file is available");
	*host = add_port_to_hostname(&delegated).into_uri_string();
	match get_ip_with_port(&delegated) {
		Some(host_and_port) => actual_dest_3_1(host_and_port),
		None => {
			if let Some(pos) = delegated.find(':') {
				actual_dest_3_2(cache, delegated, pos).await
			} else {
				trace!("Delegated hostname has no port in this branch");
				if let Some(overrider) = query_srv_record(&delegated).await? {
					actual_dest_3_3(cache, delegated, overrider).await
				} else {
					actual_dest_3_4(cache, delegated).await
				}
			}
		},
	}
}

fn actual_dest_3_1(host_and_port: FedDest) -> Result<FedDest> {
	debug!("3.1: IP literal in .well-known file");
	Ok(host_and_port)
}

async fn actual_dest_3_2(cache: bool, delegated: String, pos: usize) -> Result<FedDest> {
	debug!("3.2: Hostname with port in .well-known file");
	let (host, port) = delegated.split_at(pos);
	conditional_query_and_cache_override(host, host, port.parse::<u16>().unwrap_or(8448), cache).await?;
	Ok(FedDest::Named(host.to_owned(), port.to_owned()))
}

async fn actual_dest_3_3(cache: bool, delegated: String, overrider: FedDest) -> Result<FedDest> {
	debug!("3.3: SRV lookup successful");
	let force_port = overrider.port();
	conditional_query_and_cache_override(&delegated, &overrider.hostname(), force_port.unwrap_or(8448), cache).await?;
	if let Some(port) = force_port {
		Ok(FedDest::Named(delegated, format!(":{port}")))
	} else {
		Ok(add_port_to_hostname(&delegated))
	}
}

async fn actual_dest_3_4(cache: bool, delegated: String) -> Result<FedDest> {
	debug!("3.4: No SRV records, just use the hostname from .well-known");
	conditional_query_and_cache_override(&delegated, &delegated, 8448, cache).await?;
	Ok(add_port_to_hostname(&delegated))
}

async fn actual_dest_4(host: &str, cache: bool, overrider: FedDest) -> Result<FedDest> {
	debug!("4: No .well-known; SRV record found");
	let force_port = overrider.port();
	conditional_query_and_cache_override(host, &overrider.hostname(), force_port.unwrap_or(8448), cache).await?;
	if let Some(port) = force_port {
		Ok(FedDest::Named(host.to_owned(), format!(":{port}")))
	} else {
		Ok(add_port_to_hostname(host))
	}
}

async fn actual_dest_5(dest: &ServerName, cache: bool) -> Result<FedDest> {
	debug!("5: No SRV record found");
	conditional_query_and_cache_override(dest.as_str(), dest.as_str(), 8448, cache).await?;
	Ok(add_port_to_hostname(dest.as_str()))
}

#[tracing::instrument(skip_all, name = "well-known")]
async fn request_well_known(dest: &str) -> Result<Option<String>> {
	trace!("Requesting well known for {dest}");
	if !services().globals.resolver.has_cached_override(dest) {
		query_and_cache_override(dest, dest, 8448).await?;
	}

	let response = services()
		.globals
		.client
		.well_known
		.get(&format!("https://{dest}/.well-known/matrix/server"))
		.send()
		.await;

	trace!("response: {:?}", response);
	if let Err(e) = &response {
		debug!("error: {e:?}");
		return Ok(None);
	}

	let response = response?;
	if !response.status().is_success() {
		debug!("response not 2XX");
		return Ok(None);
	}

	let text = response.text().await?;
	trace!("response text: {:?}", text);
	if text.len() >= 12288 {
		debug_warn!("response contains junk");
		return Ok(None);
	}

	let body: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();

	let m_server = body
		.get("m.server")
		.unwrap_or(&serde_json::Value::Null)
		.as_str()
		.unwrap_or_default();

	if ruma_identifiers_validation::server_name::validate(m_server).is_err() {
		debug_error!("response content missing or invalid");
		return Ok(None);
	}

	debug_info!("{:?} found at {:?}", dest, m_server);
	Ok(Some(m_server.to_owned()))
}

#[inline]
async fn conditional_query_and_cache_override(overname: &str, hostname: &str, port: u16, cache: bool) -> Result<()> {
	if cache {
		query_and_cache_override(overname, hostname, port).await
	} else {
		Ok(())
	}
}

#[tracing::instrument(skip_all, name = "ip")]
async fn query_and_cache_override(overname: &'_ str, hostname: &'_ str, port: u16) -> Result<()> {
	match services()
		.globals
		.resolver
		.resolver
		.lookup_ip(hostname.to_owned())
		.await
	{
		Err(e) => handle_resolve_error(&e),
		Ok(override_ip) => {
			if hostname != overname {
				debug_info!("{overname:?} overriden by {hostname:?}");
			}

			services().globals.resolver.set_cached_override(
				overname.to_owned(),
				CachedOverride {
					ips: override_ip.iter().collect(),
					port,
					expire: CachedOverride::default_expire(),
				},
			);

			Ok(())
		},
	}
}

#[tracing::instrument(skip_all, name = "srv")]
async fn query_srv_record(hostname: &'_ str) -> Result<Option<FedDest>> {
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
			.resolver
			.resolver
			.srv_lookup(hostname.to_owned())
			.await
	}

	let hostnames = [format!("_matrix-fed._tcp.{hostname}."), format!("_matrix._tcp.{hostname}.")];

	for hostname in hostnames {
		match lookup_srv(&hostname).await {
			Ok(result) => return Ok(handle_successful_srv(&result)),
			Err(e) => handle_resolve_error(&e)?,
		}
	}

	Ok(None)
}

#[allow(clippy::single_match_else)]
fn handle_resolve_error(e: &ResolveError) -> Result<()> {
	use hickory_resolver::error::ResolveErrorKind;

	match *e.kind() {
		ResolveErrorKind::NoRecordsFound {
			..
		} => {
			// Raise to debug_warn if we can find out the result wasn't from cache
			debug!("{e}");
			Ok(())
		},
		_ => {
			error!("DNS {e}");
			Err(Error::Err(e.to_string()))
		},
	}
}

fn validate_dest(dest: &ServerName) -> Result<()> {
	if dest == services().globals.server_name() {
		return Err(Error::bad_config("Won't send federation request to ourselves"));
	}

	if dest.is_ip_literal() || IPAddress::is_valid(dest.host()) {
		validate_dest_ip_literal(dest)?;
	}

	Ok(())
}

fn validate_dest_ip_literal(dest: &ServerName) -> Result<()> {
	trace!("Destination is an IP literal, checking against IP range denylist.",);
	debug_assert!(
		dest.is_ip_literal() || !IPAddress::is_valid(dest.host()),
		"Destination is not an IP literal."
	);
	let ip = IPAddress::parse(dest.host()).map_err(|e| {
		debug_error!("Failed to parse IP literal from string: {}", e);
		Error::BadServerResponse("Invalid IP address")
	})?;

	validate_ip(&ip)?;

	Ok(())
}

pub(crate) fn validate_ip(ip: &IPAddress) -> Result<()> {
	if !services().globals.valid_cidr_range(ip) {
		return Err(Error::BadServerResponse("Not allowed to send requests to this IP"));
	}

	Ok(())
}

fn get_ip_with_port(dest_str: &str) -> Option<FedDest> {
	if let Ok(dest) = dest_str.parse::<SocketAddr>() {
		Some(FedDest::Literal(dest))
	} else if let Ok(ip_addr) = dest_str.parse::<IpAddr>() {
		Some(FedDest::Literal(SocketAddr::new(ip_addr, 8448)))
	} else {
		None
	}
}

fn add_port_to_hostname(dest_str: &str) -> FedDest {
	let (host, port) = match dest_str.find(':') {
		None => (dest_str, ":8448"),
		Some(pos) => dest_str.split_at(pos),
	};

	FedDest::Named(host.to_owned(), port.to_owned())
}

impl crate::globals::resolver::Resolver {
	pub(crate) fn set_cached_destination(&self, name: OwnedServerName, dest: CachedDest) -> Option<CachedDest> {
		trace!(?name, ?dest, "set cached destination");
		self.destinations
			.write()
			.expect("locked for writing")
			.insert(name, dest)
	}

	pub(crate) fn get_cached_destination(&self, name: &ServerName) -> Option<CachedDest> {
		self.destinations
			.read()
			.expect("locked for reading")
			.get(name)
			.filter(|cached| cached.valid())
			.cloned()
	}

	pub(crate) fn set_cached_override(&self, name: String, over: CachedOverride) -> Option<CachedOverride> {
		trace!(?name, ?over, "set cached override");
		self.overrides
			.write()
			.expect("locked for writing")
			.insert(name, over)
	}

	pub(crate) fn has_cached_override(&self, name: &str) -> bool {
		self.overrides
			.read()
			.expect("locked for reading")
			.get(name)
			.filter(|cached| cached.valid())
			.is_some()
	}
}

impl CachedDest {
	#[inline]
	#[must_use]
	pub fn valid(&self) -> bool { self.expire > SystemTime::now() }

	#[must_use]
	pub(crate) fn default_expire() -> SystemTime { rand::timepoint_secs(60 * 60 * 18..60 * 60 * 36) }
}

impl CachedOverride {
	#[inline]
	#[must_use]
	pub fn valid(&self) -> bool { self.expire > SystemTime::now() }

	#[must_use]
	pub(crate) fn default_expire() -> SystemTime { rand::timepoint_secs(60 * 60 * 6..60 * 60 * 12) }
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
			Self::Named(host, port) => format!("{host}{port}"),
		}
	}

	fn hostname(&self) -> String {
		match &self {
			Self::Literal(addr) => addr.ip().to_string(),
			Self::Named(host, _) => host.clone(),
		}
	}

	#[inline]
	fn port(&self) -> Option<u16> {
		match &self {
			Self::Literal(addr) => Some(addr.port()),
			Self::Named(_, port) => port[1..].parse().ok(),
		}
	}
}

impl fmt::Display for FedDest {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Named(host, port) => write!(f, "{host}{port}"),
			Self::Literal(addr) => write!(f, "{addr}"),
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
