use std::{any::Any, collections::BTreeMap, fmt::Write, sync::Arc};

use async_trait::async_trait;
use conduit::{utils::string::split_once_infallible, Result, Server};
use database::Database;

#[async_trait]
pub(crate) trait Service: Any + Send + Sync {
	/// Implement the construction of the service instance. Services are
	/// generally singletons so expect this to only be called once for a
	/// service type. Note that it may be called again after a server reload,
	/// but the prior instance will have been dropped first. Failure will
	/// shutdown the server with an error.
	fn build(args: Args<'_>) -> Result<Arc<impl Service>>
	where
		Self: Sized;

	/// Implement the service's worker loop. The service manager spawns a
	/// task and calls this function after all services have been built.
	async fn worker(self: Arc<Self>) -> Result<()> { Ok(()) }

	/// Interrupt the service. This is sent to initiate a graceful shutdown.
	/// The service worker should return from its work loop.
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

pub(crate) type Map = BTreeMap<String, MapVal>;
pub(crate) type MapVal = (Arc<dyn Service>, Arc<dyn Any + Send + Sync>);

pub(crate) fn get<T: Any + Send + Sync>(map: &Map, name: &str) -> Option<Arc<T>> {
	map.get(name).map(|(_, s)| {
		s.clone()
			.downcast::<T>()
			.expect("Service must be correctly downcast.")
	})
}

#[inline]
pub(crate) fn make_name(module_path: &str) -> &str { split_once_infallible(module_path, "::").1 }
