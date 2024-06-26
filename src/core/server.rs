use std::{
	sync::atomic::{AtomicBool, AtomicU32, Ordering},
	time::SystemTime,
};

use tokio::{runtime, sync::broadcast};

use crate::{config::Config, log, Error, Result};

/// Server runtime state; public portion
pub struct Server {
	/// Server-wide configuration instance
	pub config: Config,

	/// Timestamp server was started; used for uptime.
	pub started: SystemTime,

	/// Reload/shutdown pending indicator; server is shutting down. This is an
	/// observable used on shutdown and should not be modified.
	pub stopping: AtomicBool,

	/// Reload/shutdown desired indicator; when false, shutdown is desired. This
	/// is an observable used on shutdown and modifying is not recommended.
	pub reloading: AtomicBool,

	/// Restart desired; when true, restart it desired after shutdown.
	pub restarting: AtomicBool,

	/// Handle to the runtime
	pub runtime: Option<runtime::Handle>,

	/// Reload/shutdown signal
	pub signal: broadcast::Sender<&'static str>,

	/// Logging subsystem state
	pub log: log::Log,

	/// TODO: move stats
	pub requests_spawn_active: AtomicU32,
	pub requests_spawn_finished: AtomicU32,
	pub requests_handle_active: AtomicU32,
	pub requests_handle_finished: AtomicU32,
	pub requests_panic: AtomicU32,
}

impl Server {
	#[must_use]
	pub fn new(config: Config, runtime: Option<runtime::Handle>, log: log::Log) -> Self {
		Self {
			config,
			started: SystemTime::now(),
			stopping: AtomicBool::new(false),
			reloading: AtomicBool::new(false),
			restarting: AtomicBool::new(false),
			runtime,
			signal: broadcast::channel::<&'static str>(1).0,
			log,
			requests_spawn_active: AtomicU32::new(0),
			requests_spawn_finished: AtomicU32::new(0),
			requests_handle_active: AtomicU32::new(0),
			requests_handle_finished: AtomicU32::new(0),
			requests_panic: AtomicU32::new(0),
		}
	}

	pub fn reload(&self) -> Result<()> {
		if cfg!(not(conduit_mods)) {
			return Err(Error::Err("Reloading not enabled".into()));
		}

		if self.reloading.swap(true, Ordering::AcqRel) {
			return Err(Error::Err("Reloading already in progress".into()));
		}

		if self.stopping.swap(true, Ordering::AcqRel) {
			return Err(Error::Err("Shutdown already in progress".into()));
		}

		self.signal("SIGINT")
	}

	pub fn restart(&self) -> Result<()> {
		if self.restarting.swap(true, Ordering::AcqRel) {
			return Err(Error::Err("Restart already in progress".into()));
		}

		self.shutdown()
	}

	pub fn shutdown(&self) -> Result<()> {
		if self.stopping.swap(true, Ordering::AcqRel) {
			return Err(Error::Err("Shutdown already in progress".into()));
		}

		self.signal("SIGTERM")
	}

	pub fn signal(&self, sig: &'static str) -> Result<()> {
		if let Err(e) = self.signal.send(sig) {
			return Err(Error::Err(format!("Failed to send signal: {e}")));
		}

		Ok(())
	}

	#[inline]
	pub fn runtime(&self) -> &runtime::Handle {
		self.runtime
			.as_ref()
			.expect("runtime handle available in Server")
	}

	#[inline]
	pub fn running(&self) -> bool { !self.stopping.load(Ordering::Acquire) }
}
