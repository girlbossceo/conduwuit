use std::{sync::Arc, time::Duration};

use conduit::{Config, Result};
use reqwest::redirect;

use crate::{resolver, service};

pub struct Service {
	pub default: reqwest::Client,
	pub url_preview: reqwest::Client,
	pub well_known: reqwest::Client,
	pub federation: reqwest::Client,
	pub sender: reqwest::Client,
	pub appservice: reqwest::Client,
	pub pusher: reqwest::Client,
}

impl crate::Service for Service {
	fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
		let config = &args.server.config;
		let resolver = args
			.get_service::<resolver::Service>("resolver")
			.expect("resolver must be built prior to client");

		Ok(Arc::new(Self {
			default: base(config)
				.unwrap()
				.dns_resolver(resolver.clone())
				.build()
				.unwrap(),

			url_preview: base(config)
				.unwrap()
				.dns_resolver(resolver.clone())
				.redirect(redirect::Policy::limited(3))
				.build()
				.unwrap(),

			well_known: base(config)
				.unwrap()
				.dns_resolver(resolver.hooked.clone())
				.connect_timeout(Duration::from_secs(config.well_known_conn_timeout))
				.read_timeout(Duration::from_secs(config.well_known_timeout))
				.timeout(Duration::from_secs(config.well_known_timeout))
				.pool_max_idle_per_host(0)
				.redirect(redirect::Policy::limited(4))
				.build()
				.unwrap(),

			federation: base(config)
				.unwrap()
				.dns_resolver(resolver.hooked.clone())
				.read_timeout(Duration::from_secs(config.federation_timeout))
				.timeout(Duration::from_secs(config.federation_timeout))
				.pool_max_idle_per_host(config.federation_idle_per_host.into())
				.pool_idle_timeout(Duration::from_secs(config.federation_idle_timeout))
				.redirect(redirect::Policy::limited(3))
				.build()
				.unwrap(),

			sender: base(config)
				.unwrap()
				.dns_resolver(resolver.hooked.clone())
				.read_timeout(Duration::from_secs(config.sender_timeout))
				.timeout(Duration::from_secs(config.sender_timeout))
				.pool_max_idle_per_host(1)
				.pool_idle_timeout(Duration::from_secs(config.sender_idle_timeout))
				.redirect(redirect::Policy::limited(2))
				.build()
				.unwrap(),

			appservice: base(config)
				.unwrap()
				.dns_resolver(resolver.clone())
				.connect_timeout(Duration::from_secs(5))
				.read_timeout(Duration::from_secs(config.appservice_timeout))
				.timeout(Duration::from_secs(config.appservice_timeout))
				.pool_max_idle_per_host(1)
				.pool_idle_timeout(Duration::from_secs(config.appservice_idle_timeout))
				.redirect(redirect::Policy::limited(2))
				.build()
				.unwrap(),

			pusher: base(config)
				.unwrap()
				.dns_resolver(resolver.clone())
				.pool_max_idle_per_host(1)
				.pool_idle_timeout(Duration::from_secs(config.pusher_idle_timeout))
				.redirect(redirect::Policy::limited(2))
				.build()
				.unwrap(),
		}))
	}

	fn name(&self) -> &str { service::make_name(std::module_path!()) }
}

fn base(config: &Config) -> Result<reqwest::ClientBuilder> {
	let mut builder = reqwest::Client::builder()
		.hickory_dns(true)
		.connect_timeout(Duration::from_secs(config.request_conn_timeout))
		.read_timeout(Duration::from_secs(config.request_timeout))
		.timeout(Duration::from_secs(config.request_total_timeout))
		.pool_idle_timeout(Duration::from_secs(config.request_idle_timeout))
		.pool_max_idle_per_host(config.request_idle_per_host.into())
		.user_agent(conduit::version::user_agent())
		.redirect(redirect::Policy::limited(6))
		.connection_verbose(true);

	#[cfg(feature = "gzip_compression")]
	{
		builder = if config.gzip_compression {
			builder.gzip(true)
		} else {
			builder.gzip(false).no_gzip()
		};
	};

	#[cfg(feature = "brotli_compression")]
	{
		builder = if config.brotli_compression {
			builder.brotli(true)
		} else {
			builder.brotli(false).no_brotli()
		};
	};

	#[cfg(not(feature = "gzip_compression"))]
	{
		builder = builder.no_gzip();
	};

	#[cfg(not(feature = "brotli_compression"))]
	{
		builder = builder.no_brotli();
	};

	if let Some(proxy) = config.proxy.to_proxy()? {
		Ok(builder.proxy(proxy))
	} else {
		Ok(builder)
	}
}
