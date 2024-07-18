pub mod actual;
pub mod cache;
mod dns;
pub mod fed;
mod tests;

use std::{fmt::Write, sync::Arc};

use conduit::{Result, Server};
use hickory_resolver::TokioAsyncResolver;

use self::{cache::Cache, dns::Resolver};
use crate::{client, globals, Dep};

pub struct Service {
	pub cache: Arc<Cache>,
	pub resolver: Arc<Resolver>,
	services: Services,
}

struct Services {
	server: Arc<Server>,
	client: Dep<client::Service>,
	globals: Dep<globals::Service>,
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
				globals: args.depend::<globals::Service>("globals"),
			},
		}))
	}

	fn memory_usage(&self, out: &mut dyn Write) -> Result<()> {
		let resolver_overrides_cache = self
			.cache
			.overrides
			.read()
			.expect("locked for reading")
			.len();
		writeln!(out, "resolver_overrides_cache: {resolver_overrides_cache}")?;

		let resolver_destinations_cache = self
			.cache
			.destinations
			.read()
			.expect("locked for reading")
			.len();
		writeln!(out, "resolver_destinations_cache: {resolver_destinations_cache}")?;

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

impl Service {
	#[inline]
	#[must_use]
	pub fn raw(&self) -> Arc<TokioAsyncResolver> { self.resolver.resolver.clone() }
}
