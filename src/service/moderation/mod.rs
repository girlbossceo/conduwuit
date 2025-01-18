use std::sync::Arc;

use conduwuit::{implement, Result, Server};
use ruma::ServerName;

use crate::{globals, Dep};

pub struct Service {
	services: Services,
}

struct Services {
	pub server: Arc<Server>,
	pub globals: Dep<globals::Service>,
}

impl crate::Service for Service {
	fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
		Ok(Arc::new(Self {
			services: Services {
				server: args.server.clone(),
				globals: args.depend::<globals::Service>("globals"),
			},
		}))
	}

	fn name(&self) -> &str { crate::service::make_name(std::module_path!()) }
}

#[implement(Service)]
#[must_use]
pub fn is_remote_server_forbidden(&self, server_name: &ServerName) -> bool {
	// Forbidden if NOT (allowed is empty OR allowed contains server OR is self)
	// OR forbidden contains server
	!(self
		.services
		.server
		.config
		.allowed_remote_server_names
		.is_empty()
		|| self
			.services
			.server
			.config
			.allowed_remote_server_names
			.contains(server_name)
		|| server_name == self.services.globals.server_name())
		|| self
			.services
			.server
			.config
			.forbidden_remote_server_names
			.contains(server_name)
}
