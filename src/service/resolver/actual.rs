use std::{
	fmt::Debug,
	net::{IpAddr, SocketAddr},
	sync::Arc,
};

use conduit::{debug, debug_error, debug_info, debug_warn, err, trace, Err, Result};
use hickory_resolver::{error::ResolveError, lookup::SrvLookup};
use ipaddress::IPAddress;
use ruma::ServerName;

use crate::resolver::{
	cache::{CachedDest, CachedOverride},
	fed::{add_port_to_hostname, get_ip_with_port, FedDest},
};

#[derive(Clone, Debug)]
pub(crate) struct ActualDest {
	pub(crate) dest: FedDest,
	pub(crate) host: String,
	pub(crate) string: String,
	pub(crate) cached: bool,
}

impl super::Service {
	#[tracing::instrument(skip_all, name = "resolve")]
	pub(crate) async fn get_actual_dest(&self, server_name: &ServerName) -> Result<ActualDest> {
		let cached;
		let cached_result = self.get_cached_destination(server_name);

		let CachedDest {
			dest,
			host,
			..
		} = if let Some(result) = cached_result {
			cached = true;
			result
		} else {
			cached = false;
			self.validate_dest(server_name)?;
			self.resolve_actual_dest(server_name, true).await?
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
	pub async fn resolve_actual_dest(&self, dest: &ServerName, cache: bool) -> Result<CachedDest> {
		trace!("Finding actual destination for {dest}");
		let mut host = dest.as_str().to_owned();
		let actual_dest = match get_ip_with_port(dest.as_str()) {
			Some(host_port) => Self::actual_dest_1(host_port)?,
			None => {
				if let Some(pos) = dest.as_str().find(':') {
					self.actual_dest_2(dest, cache, pos).await?
				} else if let Some(delegated) = self.request_well_known(dest.as_str()).await? {
					self.actual_dest_3(&mut host, cache, delegated).await?
				} else if let Some(overrider) = self.query_srv_record(dest.as_str()).await? {
					self.actual_dest_4(&host, cache, overrider).await?
				} else {
					self.actual_dest_5(dest, cache).await?
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

	async fn actual_dest_2(&self, dest: &ServerName, cache: bool, pos: usize) -> Result<FedDest> {
		debug!("2: Hostname with included port");
		let (host, port) = dest.as_str().split_at(pos);
		self.conditional_query_and_cache_override(host, host, port.parse::<u16>().unwrap_or(8448), cache)
			.await?;
		Ok(FedDest::Named(host.to_owned(), port.to_owned()))
	}

	async fn actual_dest_3(&self, host: &mut String, cache: bool, delegated: String) -> Result<FedDest> {
		debug!("3: A .well-known file is available");
		*host = add_port_to_hostname(&delegated).into_uri_string();
		match get_ip_with_port(&delegated) {
			Some(host_and_port) => Self::actual_dest_3_1(host_and_port),
			None => {
				if let Some(pos) = delegated.find(':') {
					self.actual_dest_3_2(cache, delegated, pos).await
				} else {
					trace!("Delegated hostname has no port in this branch");
					if let Some(overrider) = self.query_srv_record(&delegated).await? {
						self.actual_dest_3_3(cache, delegated, overrider).await
					} else {
						self.actual_dest_3_4(cache, delegated).await
					}
				}
			},
		}
	}

	fn actual_dest_3_1(host_and_port: FedDest) -> Result<FedDest> {
		debug!("3.1: IP literal in .well-known file");
		Ok(host_and_port)
	}

	async fn actual_dest_3_2(&self, cache: bool, delegated: String, pos: usize) -> Result<FedDest> {
		debug!("3.2: Hostname with port in .well-known file");
		let (host, port) = delegated.split_at(pos);
		self.conditional_query_and_cache_override(host, host, port.parse::<u16>().unwrap_or(8448), cache)
			.await?;
		Ok(FedDest::Named(host.to_owned(), port.to_owned()))
	}

	async fn actual_dest_3_3(&self, cache: bool, delegated: String, overrider: FedDest) -> Result<FedDest> {
		debug!("3.3: SRV lookup successful");
		let force_port = overrider.port();
		self.conditional_query_and_cache_override(&delegated, &overrider.hostname(), force_port.unwrap_or(8448), cache)
			.await?;
		if let Some(port) = force_port {
			Ok(FedDest::Named(delegated, format!(":{port}")))
		} else {
			Ok(add_port_to_hostname(&delegated))
		}
	}

	async fn actual_dest_3_4(&self, cache: bool, delegated: String) -> Result<FedDest> {
		debug!("3.4: No SRV records, just use the hostname from .well-known");
		self.conditional_query_and_cache_override(&delegated, &delegated, 8448, cache)
			.await?;
		Ok(add_port_to_hostname(&delegated))
	}

	async fn actual_dest_4(&self, host: &str, cache: bool, overrider: FedDest) -> Result<FedDest> {
		debug!("4: No .well-known; SRV record found");
		let force_port = overrider.port();
		self.conditional_query_and_cache_override(host, &overrider.hostname(), force_port.unwrap_or(8448), cache)
			.await?;
		if let Some(port) = force_port {
			Ok(FedDest::Named(host.to_owned(), format!(":{port}")))
		} else {
			Ok(add_port_to_hostname(host))
		}
	}

	async fn actual_dest_5(&self, dest: &ServerName, cache: bool) -> Result<FedDest> {
		debug!("5: No SRV record found");
		self.conditional_query_and_cache_override(dest.as_str(), dest.as_str(), 8448, cache)
			.await?;
		Ok(add_port_to_hostname(dest.as_str()))
	}

	#[tracing::instrument(skip_all, name = "well-known")]
	async fn request_well_known(&self, dest: &str) -> Result<Option<String>> {
		trace!("Requesting well known for {dest}");
		if !self.has_cached_override(dest) {
			self.query_and_cache_override(dest, dest, 8448).await?;
		}

		let response = self
			.services
			.client
			.well_known
			.get(format!("https://{dest}/.well-known/matrix/server"))
			.send()
			.await;

		trace!("response: {response:?}");
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
		trace!("response text: {text:?}");
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

		if ruma::identifiers_validation::server_name::validate(m_server).is_err() {
			debug_error!("response content missing or invalid");
			return Ok(None);
		}

		debug_info!("{dest:?} found at {m_server:?}");
		Ok(Some(m_server.to_owned()))
	}

	#[inline]
	async fn conditional_query_and_cache_override(
		&self, overname: &str, hostname: &str, port: u16, cache: bool,
	) -> Result<()> {
		if cache {
			self.query_and_cache_override(overname, hostname, port)
				.await
		} else {
			Ok(())
		}
	}

	#[tracing::instrument(skip_all, name = "ip")]
	async fn query_and_cache_override(&self, overname: &'_ str, hostname: &'_ str, port: u16) -> Result<()> {
		match self.raw().lookup_ip(hostname.to_owned()).await {
			Err(e) => Self::handle_resolve_error(&e),
			Ok(override_ip) => {
				if hostname != overname {
					debug_info!("{overname:?} overriden by {hostname:?}");
				}

				self.set_cached_override(
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
	async fn query_srv_record(&self, hostname: &'_ str) -> Result<Option<FedDest>> {
		fn handle_successful_srv(srv: &SrvLookup) -> Option<FedDest> {
			srv.iter().next().map(|result| {
				FedDest::Named(
					result.target().to_string().trim_end_matches('.').to_owned(),
					format!(":{}", result.port()),
				)
			})
		}

		async fn lookup_srv(
			resolver: Arc<super::TokioAsyncResolver>, hostname: &str,
		) -> Result<SrvLookup, ResolveError> {
			debug!("querying SRV for {hostname:?}");
			let hostname = hostname.trim_end_matches('.');
			resolver.srv_lookup(hostname.to_owned()).await
		}

		let hostnames = [format!("_matrix-fed._tcp.{hostname}."), format!("_matrix._tcp.{hostname}.")];

		for hostname in hostnames {
			match lookup_srv(self.raw(), &hostname).await {
				Ok(result) => return Ok(handle_successful_srv(&result)),
				Err(e) => Self::handle_resolve_error(&e)?,
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
			_ => Err!(error!("DNS {e}")),
		}
	}

	fn validate_dest(&self, dest: &ServerName) -> Result<()> {
		let config = &self.services.server.config;
		if dest == config.server_name && !config.federation_loopback {
			return Err!("Won't send federation request to ourselves");
		}

		if dest.is_ip_literal() || IPAddress::is_valid(dest.host()) {
			self.validate_dest_ip_literal(dest)?;
		}

		Ok(())
	}

	fn validate_dest_ip_literal(&self, dest: &ServerName) -> Result<()> {
		trace!("Destination is an IP literal, checking against IP range denylist.",);
		debug_assert!(
			dest.is_ip_literal() || !IPAddress::is_valid(dest.host()),
			"Destination is not an IP literal."
		);
		let ip = IPAddress::parse(dest.host())
			.map_err(|e| err!(BadServerResponse(debug_error!("Failed to parse IP literal from string: {e}"))))?;

		self.validate_ip(&ip)?;

		Ok(())
	}

	pub(crate) fn validate_ip(&self, ip: &IPAddress) -> Result<()> {
		if !self.services.globals.valid_cidr_range(ip) {
			return Err!(BadServerResponse("Not allowed to send requests to this IP"));
		}

		Ok(())
	}
}
