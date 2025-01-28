use std::{iter, ops::Deref, path::Path, sync::Arc};

use async_trait::async_trait;
use conduwuit::{
	config::{check, Config},
	error, implement, Result, Server,
};

pub struct Service {
	server: Arc<Server>,
}

const SIGNAL: &str = "SIGUSR1";

#[async_trait]
impl crate::Service for Service {
	fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
		Ok(Arc::new(Self { server: args.server.clone() }))
	}

	async fn worker(self: Arc<Self>) -> Result {
		while self.server.running() {
			if self.server.signal.subscribe().recv().await == Ok(SIGNAL) {
				if let Err(e) = self.handle_reload() {
					error!("Failed to reload config: {e}");
				}
			}
		}

		Ok(())
	}

	fn name(&self) -> &str { crate::service::make_name(std::module_path!()) }
}

impl Deref for Service {
	type Target = Arc<Config>;

	#[inline]
	fn deref(&self) -> &Self::Target { &self.server.config }
}

#[implement(Service)]
fn handle_reload(&self) -> Result {
	if self.server.config.config_reload_signal {
		self.reload(iter::empty())?;
	}

	Ok(())
}

#[implement(Service)]
pub fn reload<'a, I>(&self, paths: I) -> Result<Arc<Config>>
where
	I: Iterator<Item = &'a Path>,
{
	let old = self.server.config.clone();
	let new = Config::load(paths).and_then(|raw| Config::new(&raw))?;

	check::reload(&old, &new)?;
	self.server.config.update(new)
}
