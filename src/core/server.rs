use std::{
	sync::{
		atomic::{AtomicBool, AtomicU32},
		Mutex,
	},
	time::SystemTime,
};

use tokio::runtime;

use crate::{config::Config, log::LogLevelReloadHandles};

/// Server runtime state; public portion
pub struct Server {
	/// Server-wide configuration instance
	pub config: Config,

	/// Timestamp server was started; used for uptime.
	pub started: SystemTime,

	/// Reload/shutdown signal channel. Called from the signal handler or admin
	/// command to initiate shutdown.
	pub shutdown: Mutex<Option<axum_server::Handle>>,

	/// Reload/shutdown desired indicator; when false, shutdown is desired. This
	/// is an observable used on shutdown and modifying is not recommended.
	pub reload: AtomicBool,

	/// Reload/shutdown pending indicator; server is shutting down. This is an
	/// observable used on shutdown and should not be modified.
	pub interrupt: AtomicBool,

	/// Handle to the runtime
	pub runtime: Option<runtime::Handle>,

	/// Log level reload handles.
	pub tracing_reload_handle: LogLevelReloadHandles,

	/// TODO: move stats
	pub requests_spawn_active: AtomicU32,
	pub requests_spawn_finished: AtomicU32,
	pub requests_handle_active: AtomicU32,
	pub requests_handle_finished: AtomicU32,
	pub requests_panic: AtomicU32,
}

impl Server {
	#[must_use]
	pub fn new(config: Config, runtime: Option<runtime::Handle>, tracing_reload_handle: LogLevelReloadHandles) -> Self {
		Self {
			config,
			started: SystemTime::now(),
			shutdown: Mutex::new(None),
			reload: AtomicBool::new(false),
			interrupt: AtomicBool::new(false),
			runtime,
			tracing_reload_handle,
			requests_spawn_active: AtomicU32::new(0),
			requests_spawn_finished: AtomicU32::new(0),
			requests_handle_active: AtomicU32::new(0),
			requests_handle_finished: AtomicU32::new(0),
			requests_panic: AtomicU32::new(0),
		}
	}

	#[inline]
	pub fn runtime(&self) -> &runtime::Handle {
		self.runtime
			.as_ref()
			.expect("runtime handle available in Server")
	}
}
