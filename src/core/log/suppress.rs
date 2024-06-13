use std::sync::Arc;

use super::EnvFilter;
use crate::Server;

pub struct Suppress {
	server: Arc<Server>,
	restore: EnvFilter,
}

impl Suppress {
	pub fn new(server: &Arc<Server>) -> Self {
		let config = &server.config.log;
		Self::from_filters(server, EnvFilter::try_new(config).unwrap_or_default(), &EnvFilter::default())
	}

	fn from_filters(server: &Arc<Server>, restore: EnvFilter, suppress: &EnvFilter) -> Self {
		server
			.log
			.reload
			.reload(suppress)
			.expect("log filter reloaded");
		Self {
			server: server.clone(),
			restore,
		}
	}
}

impl Drop for Suppress {
	fn drop(&mut self) {
		self.server
			.log
			.reload
			.reload(&self.restore)
			.expect("log filter reloaded");
	}
}
