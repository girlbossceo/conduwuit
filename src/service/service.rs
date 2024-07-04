use std::{collections::BTreeMap, fmt::Write, sync::Arc};

use async_trait::async_trait;
use conduit::{utils::string::split_once_infallible, Result, Server};
use database::Database;

#[async_trait]
pub(crate) trait Service: Send + Sync {
	/// Implement the construction of the service instance. Services are
	/// generally singletons so expect this to only be called once for a
	/// service type. Note that it may be called again after a server reload,
	/// but the prior instance will have been dropped first. Failure will
	/// shutdown the server with an error.
	fn build(args: Args<'_>) -> Result<Arc<impl Service>>
	where
		Self: Sized;

	/// Start the service. Implement the spawning of any service workers. This
	/// is called after all other services have been constructed. Failure will
	/// shutdown the server with an error.
	async fn start(self: Arc<Self>) -> Result<()> { Ok(()) }

	/// Stop the service. Implement the joining of any service workers and
	/// cleanup of any other state. This function is asynchronous to allow that
	/// gracefully, but errors cannot propagate.
	async fn stop(&self) {}

	/// Interrupt the service. This may be sent prior to `stop()` as a
	/// notification to improve the shutdown sequence. Implementations must be
	/// robust to this being called multiple times.
	fn interrupt(&self) {}

	/// Clear any caches or similar runtime state.
	fn clear_cache(&self) {}

	/// Memory usage report in a markdown string.
	fn memory_usage(&self, _out: &mut dyn Write) -> Result<()> { Ok(()) }

	/// Return the name of the service.
	/// i.e. `crate::service::make_name(std::module_path!())`
	fn name(&self) -> &str;
}

pub(crate) struct Args<'a> {
	pub(crate) server: &'a Arc<Server>,
	pub(crate) db: &'a Arc<Database>,
	pub(crate) _service: &'a Map,
}

pub(crate) type Map = BTreeMap<String, Arc<dyn Service>>;

#[inline]
pub(crate) fn make_name(module_path: &str) -> &str { split_once_infallible(module_path, "::").1 }
