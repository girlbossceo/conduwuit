pub mod actual;
pub mod cache;
mod dns;
pub mod fed;
mod tests;

use std::{fmt::Write, sync::Arc};

use conduwuit::{utils, utils::math::Expected, Result, Server};

use self::{cache::Cache, dns::Resolver};
use crate::{client, Dep};

pub struct Service {
	pub cache: Arc<Cache>,
	pub resolver: Arc<Resolver>,
	services: Services,
}

struct Services {
	server: Arc<Server>,
	client: Dep<client::Service>,
}

impl crate::Service for Service {
	#[allow(clippy::as_conversions, clippy::cast_sign_loss, clippy::cast_possible_truncation)]
	fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
		let cache = Cache::new();
		Ok(Arc::new(Self {
			cache: cache.clone(),
			resolver: Resolver::build(args.server, cache)?,
			services: Services {
				server: args.server.clone(),
				client: args.depend::<client::Service>("client"),
			},
		}))
	}

	fn memory_usage(&self, out: &mut dyn Write) -> Result {
		use utils::bytes::pretty;

		let (oc_count, oc_bytes) = self.cache.overrides.read()?.iter().fold(
			(0_usize, 0_usize),
			|(count, bytes), (key, val)| {
				(count.expected_add(1), bytes.expected_add(key.len()).expected_add(val.size()))
			},
		);

		let (dc_count, dc_bytes) = self.cache.destinations.read()?.iter().fold(
			(0_usize, 0_usize),
			|(count, bytes), (key, val)| {
				(count.expected_add(1), bytes.expected_add(key.len()).expected_add(val.size()))
			},
		);

		writeln!(out, "resolver_overrides_cache: {oc_count} ({})", pretty(oc_bytes))?;
		writeln!(out, "resolver_destinations_cache: {dc_count} ({})", pretty(dc_bytes))?;

		Ok(())
	}

	fn clear_cache(&self) {
		self.cache.overrides.write().expect("write locked").clear();
		self.cache
			.destinations
			.write()
			.expect("write locked")
			.clear();
		self.resolver.resolver.clear_cache();
	}

	fn name(&self) -> &str { crate::service::make_name(std::module_path!()) }
}
