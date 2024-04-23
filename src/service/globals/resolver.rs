use std::{
	collections::HashMap,
	future, iter,
	net::{IpAddr, SocketAddr},
	sync::{Arc, RwLock as StdRwLock},
	time::Duration,
};

use hickory_resolver::TokioAsyncResolver;
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use ruma::OwnedServerName;
use tokio::sync::RwLock;
use tracing::error;

use crate::{service::sending::FedDest, Config, Error};

pub(crate) type WellKnownMap = HashMap<OwnedServerName, (FedDest, String)>;
type TlsNameMap = HashMap<String, (Vec<IpAddr>, u16)>;

pub(crate) struct Resolver {
	pub(crate) destinations: Arc<RwLock<WellKnownMap>>, // actual_destination, host
	pub(crate) overrides: Arc<StdRwLock<TlsNameMap>>,
	pub(crate) resolver: Arc<TokioAsyncResolver>,
	pub(crate) hooked: Arc<Hooked>,
}

pub(crate) struct Hooked {
	pub(crate) overrides: Arc<StdRwLock<TlsNameMap>>,
	pub(crate) resolver: Arc<TokioAsyncResolver>,
}

impl Resolver {
	pub(crate) fn new(config: &Config) -> Self {
		let (sys_conf, mut opts) = hickory_resolver::system_conf::read_system_conf()
			.map_err(|e| {
				error!("Failed to set up hickory dns resolver with system config: {}", e);
				Error::bad_config("Failed to set up hickory dns resolver with system config.")
			})
			.unwrap();

		let mut conf = hickory_resolver::config::ResolverConfig::new();

		if let Some(domain) = sys_conf.domain() {
			conf.set_domain(domain.clone());
		}

		for sys_conf in sys_conf.search() {
			conf.add_search(sys_conf.clone());
		}

		for sys_conf in sys_conf.name_servers() {
			let mut ns = sys_conf.clone();

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

		let resolver = Arc::new(TokioAsyncResolver::tokio(conf, opts));
		let overrides = Arc::new(StdRwLock::new(TlsNameMap::new()));
		Resolver {
			destinations: Arc::new(RwLock::new(WellKnownMap::new())),
			overrides: overrides.clone(),
			resolver: resolver.clone(),
			hooked: Arc::new(Hooked {
				overrides,
				resolver,
			}),
		}
	}
}

impl Resolve for Resolver {
	fn resolve(&self, name: Name) -> Resolving { resolve_to_reqwest(self.resolver.clone(), name) }
}

impl Resolve for Hooked {
	fn resolve(&self, name: Name) -> Resolving {
		let addr_port = self.overrides.read().unwrap().get(name.as_str()).cloned();
		if let Some((addr, port)) = addr_port {
			cached_to_reqwest(&addr, port)
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
		.unwrap()
}

fn resolve_to_reqwest(resolver: Arc<TokioAsyncResolver>, name: Name) -> Resolving {
	Box::pin(async move {
		let results = resolver
			.lookup_ip(name.as_str())
			.await?
			.into_iter()
			.map(|ip| SocketAddr::new(ip, 0));

		Ok(Box::new(results) as Addrs)
	})
}
