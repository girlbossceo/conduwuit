pub mod actual;
pub mod cache;
mod dns;
pub mod fed;
mod tests;

use std::sync::Arc;

use conduwuit::{Result, Server};

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
		let cache = Cache::new(&args);
		Ok(Arc::new(Self {
			cache: cache.clone(),
			resolver: Resolver::build(args.server, cache)?,
			services: Services {
				server: args.server.clone(),
				client: args.depend::<client::Service>("client"),
			},
		}))
	}

	fn name(&self) -> &str { crate::service::make_name(std::module_path!()) }
}
