use std::sync::Arc;

use super::EnvFilter;
use crate::Server;

pub struct Suppress {
	server: Arc<Server>,
	restore: EnvFilter,
}

impl Suppress {
	pub fn new(server: &Arc<Server>) -> Self {
		let handle = "console";
		let config = &server.config.log;
		let suppress = EnvFilter::default();
		let restore = server
			.log
			.reload
			.current(handle)
			.unwrap_or_else(|| EnvFilter::try_new(config).unwrap_or_default());

		server
			.log
			.reload
			.reload(&suppress, Some(&[handle]))
			.expect("log filter reloaded");

		Self { server: server.clone(), restore }
	}
}

impl Drop for Suppress {
	fn drop(&mut self) {
		self.server
			.log
			.reload
			.reload(&self.restore, Some(&["console"]))
			.expect("log filter reloaded");
	}
}
