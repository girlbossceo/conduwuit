use std::{
	collections::HashMap,
	fmt,
	fmt::Write,
	future, iter,
	net::{IpAddr, SocketAddr},
	sync::{Arc, RwLock},
	time::{Duration, SystemTime},
};

use conduit::{err, trace, Result};
use hickory_resolver::TokioAsyncResolver;
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use ruma::{OwnedServerName, ServerName};

use crate::utils::rand;

pub struct Service {
	pub destinations: Arc<RwLock<WellKnownMap>>, // actual_destination, host
	pub overrides: Arc<RwLock<TlsNameMap>>,
	pub(crate) resolver: Arc<TokioAsyncResolver>,
	pub(crate) hooked: Arc<Hooked>,
}

pub(crate) struct Hooked {
	overrides: Arc<RwLock<TlsNameMap>>,
	resolver: Arc<TokioAsyncResolver>,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FedDest {
	Literal(SocketAddr),
	Named(String, String),
}

type WellKnownMap = HashMap<OwnedServerName, CachedDest>;
type TlsNameMap = HashMap<String, CachedOverride>;

impl crate::Service for Service {
	#[allow(clippy::as_conversions, clippy::cast_sign_loss, clippy::cast_possible_truncation)]
	fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
		let config = &args.server.config;
		let (sys_conf, mut opts) = hickory_resolver::system_conf::read_system_conf()
			.map_err(|e| err!(error!("Failed to configure DNS resolver from system: {e}")))?;

		let mut conf = hickory_resolver::config::ResolverConfig::new();

		if let Some(domain) = sys_conf.domain() {
			conf.set_domain(domain.clone());
		}

		for sys_conf in sys_conf.search() {
			conf.add_search(sys_conf.clone());
		}

		for sys_conf in sys_conf.name_servers() {
			let mut ns = sys_conf.clone();

			if config.query_over_tcp_only {
				ns.protocol = hickory_resolver::config::Protocol::Tcp;
			}

			ns.trust_negative_responses = !config.query_all_nameservers;

			conf.add_name_server(ns);
		}

		opts.cache_size = config.dns_cache_entries as usize;
		opts.negative_min_ttl = Some(Duration::from_secs(config.dns_min_ttl_nxdomain));
		opts.negative_max_ttl = Some(Duration::from_secs(60 * 60 * 24 * 30));
		opts.positive_min_ttl = Some(Duration::from_secs(config.dns_min_ttl));
		opts.positive_max_ttl = Some(Duration::from_secs(60 * 60 * 24 * 7));
		opts.timeout = Duration::from_secs(config.dns_timeout);
		opts.attempts = config.dns_attempts as usize;
		opts.try_tcp_on_error = config.dns_tcp_fallback;
		opts.num_concurrent_reqs = 1;
		opts.shuffle_dns_servers = true;
		opts.rotate = true;
		opts.ip_strategy = match config.ip_lookup_strategy {
			1 => hickory_resolver::config::LookupIpStrategy::Ipv4Only,
			2 => hickory_resolver::config::LookupIpStrategy::Ipv6Only,
			3 => hickory_resolver::config::LookupIpStrategy::Ipv4AndIpv6,
			4 => hickory_resolver::config::LookupIpStrategy::Ipv6thenIpv4,
			_ => hickory_resolver::config::LookupIpStrategy::Ipv4thenIpv6,
		};
		opts.authentic_data = false;

		let resolver = Arc::new(TokioAsyncResolver::tokio(conf, opts));
		let overrides = Arc::new(RwLock::new(TlsNameMap::new()));
		Ok(Arc::new(Self {
			destinations: Arc::new(RwLock::new(WellKnownMap::new())),
			overrides: overrides.clone(),
			resolver: resolver.clone(),
			hooked: Arc::new(Hooked {
				overrides,
				resolver,
			}),
		}))
	}

	fn memory_usage(&self, out: &mut dyn Write) -> Result<()> {
		let resolver_overrides_cache = self.overrides.read().expect("locked for reading").len();
		writeln!(out, "resolver_overrides_cache: {resolver_overrides_cache}")?;

		let resolver_destinations_cache = self.destinations.read().expect("locked for reading").len();
		writeln!(out, "resolver_destinations_cache: {resolver_destinations_cache}")?;

		Ok(())
	}

	fn clear_cache(&self) {
		self.overrides.write().expect("write locked").clear();
		self.destinations.write().expect("write locked").clear();
		self.resolver.clear_cache();
	}

	fn name(&self) -> &str { crate::service::make_name(std::module_path!()) }
}

impl Service {
	pub fn set_cached_destination(&self, name: OwnedServerName, dest: CachedDest) -> Option<CachedDest> {
		trace!(?name, ?dest, "set cached destination");
		self.destinations
			.write()
			.expect("locked for writing")
			.insert(name, dest)
	}

	#[must_use]
	pub fn get_cached_destination(&self, name: &ServerName) -> Option<CachedDest> {
		self.destinations
			.read()
			.expect("locked for reading")
			.get(name)
			.cloned()
	}

	pub fn set_cached_override(&self, name: String, over: CachedOverride) -> Option<CachedOverride> {
		trace!(?name, ?over, "set cached override");
		self.overrides
			.write()
			.expect("locked for writing")
			.insert(name, over)
	}

	#[must_use]
	pub fn get_cached_override(&self, name: &str) -> Option<CachedOverride> {
		self.overrides
			.read()
			.expect("locked for reading")
			.get(name)
			.cloned()
	}

	#[must_use]
	pub fn has_cached_override(&self, name: &str) -> bool {
		self.overrides
			.read()
			.expect("locked for reading")
			.contains_key(name)
	}
}

impl Resolve for Service {
	fn resolve(&self, name: Name) -> Resolving { resolve_to_reqwest(self.resolver.clone(), name) }
}

impl Resolve for Hooked {
	fn resolve(&self, name: Name) -> Resolving {
		let cached = self
			.overrides
			.read()
			.expect("locked for reading")
			.get(name.as_str())
			.cloned();

		if let Some(cached) = cached {
			cached_to_reqwest(&cached.ips, cached.port)
		} else {
			resolve_to_reqwest(self.resolver.clone(), name)
		}
	}
}

fn cached_to_reqwest(override_name: &[IpAddr], port: u16) -> Resolving {
	override_name
		.first()
		.map(|first_name| -> Resolving {
			let saddr = SocketAddr::new(*first_name, port);
			let result: Box<dyn Iterator<Item = SocketAddr> + Send> = Box::new(iter::once(saddr));
			Box::pin(future::ready(Ok(result)))
		})
		.expect("must provide at least one override name")
}

fn resolve_to_reqwest(resolver: Arc<TokioAsyncResolver>, name: Name) -> Resolving {
	Box::pin(async move {
		let results = resolver
			.lookup_ip(name.as_str())
			.await?
			.into_iter()
			.map(|ip| SocketAddr::new(ip, 0));

		let results: Addrs = Box::new(results);

		Ok(results)
	})
}

impl CachedDest {
	#[inline]
	#[must_use]
	pub fn valid(&self) -> bool { true }

	//pub fn valid(&self) -> bool { self.expire > SystemTime::now() }

	#[must_use]
	pub(crate) fn default_expire() -> SystemTime { rand::timepoint_secs(60 * 60 * 18..60 * 60 * 36) }
}

impl CachedOverride {
	#[inline]
	#[must_use]
	pub fn valid(&self) -> bool { true }

	//pub fn valid(&self) -> bool { self.expire > SystemTime::now() }

	#[must_use]
	pub(crate) fn default_expire() -> SystemTime { rand::timepoint_secs(60 * 60 * 6..60 * 60 * 12) }
}

pub(crate) fn get_ip_with_port(dest_str: &str) -> Option<FedDest> {
	if let Ok(dest) = dest_str.parse::<SocketAddr>() {
		Some(FedDest::Literal(dest))
	} else if let Ok(ip_addr) = dest_str.parse::<IpAddr>() {
		Some(FedDest::Literal(SocketAddr::new(ip_addr, 8448)))
	} else {
		None
	}
}

pub(crate) fn add_port_to_hostname(dest_str: &str) -> FedDest {
	let (host, port) = match dest_str.find(':') {
		None => (dest_str, ":8448"),
		Some(pos) => dest_str.split_at(pos),
	};

	FedDest::Named(host.to_owned(), port.to_owned())
}

impl FedDest {
	pub(crate) fn into_https_string(self) -> String {
		match self {
			Self::Literal(addr) => format!("https://{addr}"),
			Self::Named(host, port) => format!("https://{host}{port}"),
		}
	}

	pub(crate) fn into_uri_string(self) -> String {
		match self {
			Self::Literal(addr) => addr.to_string(),
			Self::Named(host, port) => format!("{host}{port}"),
		}
	}

	pub(crate) fn hostname(&self) -> String {
		match &self {
			Self::Literal(addr) => addr.ip().to_string(),
			Self::Named(host, _) => host.clone(),
		}
	}

	#[inline]
	#[allow(clippy::string_slice)]
	pub(crate) fn port(&self) -> Option<u16> {
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
