use std::{
	any::Any,
	collections::BTreeMap,
	fmt::Write,
	ops::Deref,
	sync::{Arc, OnceLock},
};

use async_trait::async_trait;
use conduit::{err, error::inspect_log, utils::string::split_once_infallible, Err, Result, Server};
use database::Database;

/// Abstract interface for a Service
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

/// Args are passed to `Service::build` when a service is constructed. This
/// allows for arguments to change with limited impact to the many services.
pub(crate) struct Args<'a> {
	pub(crate) server: &'a Arc<Server>,
	pub(crate) db: &'a Arc<Database>,
	pub(crate) service: &'a Arc<Map>,
}

/// Dep is a reference to a service used within another service.
/// Circular-dependencies between services require this indirection to allow the
/// referenced service construction after the referencing service.
pub(crate) struct Dep<T> {
	dep: OnceLock<Arc<T>>,
	service: Arc<Map>,
	name: &'static str,
}

pub(crate) type Map = BTreeMap<String, MapVal>;
pub(crate) type MapVal = (Arc<dyn Service>, Arc<dyn Any + Send + Sync>);

impl<T: Any + Send + Sync> Deref for Dep<T> {
	type Target = Arc<T>;

	fn deref(&self) -> &Self::Target {
		self.dep
			.get_or_init(|| require::<T>(&self.service, self.name))
	}
}

impl Args<'_> {
	pub(crate) fn depend_service<T: Any + Send + Sync>(&self, name: &'static str) -> Dep<T> {
		Dep::<T> {
			dep: OnceLock::new(),
			service: self.service.clone(),
			name,
		}
	}
}

pub(crate) fn require<T: Any + Send + Sync>(map: &Map, name: &str) -> Arc<T> {
	try_get::<T>(map, name)
		.inspect_err(inspect_log)
		.expect("Failure to reference service required by another service.")
}

pub(crate) fn try_get<T: Any + Send + Sync>(map: &Map, name: &str) -> Result<Arc<T>> {
	map.get(name).map_or_else(
		|| Err!("Service {name:?} does not exist or has not been built yet."),
		|(_, s)| {
			s.clone()
				.downcast::<T>()
				.map_err(|_| err!("Service {name:?} must be correctly downcast."))
		},
	)
}

pub(crate) fn get<T: Any + Send + Sync>(map: &Map, name: &str) -> Option<Arc<T>> {
	map.get(name).map(|(_, s)| {
		s.clone()
			.downcast::<T>()
			.expect("Service must be correctly downcast.")
	})
}

#[inline]
pub(crate) fn make_name(module_path: &str) -> &str { split_once_infallible(module_path, "::").1 }
