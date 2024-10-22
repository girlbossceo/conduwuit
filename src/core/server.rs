use std::{
	sync::atomic::{AtomicBool, Ordering},
	time::SystemTime,
};

use tokio::{runtime, sync::broadcast};

use crate::{config::Config, err, log::Log, metrics::Metrics, Err, Result};

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
	pub log: Log,

	/// Metrics subsystem state
	pub metrics: Metrics,
}

impl Server {
	#[must_use]
	pub fn new(config: Config, runtime: Option<runtime::Handle>, log: Log) -> Self {
		Self {
			config,
			started: SystemTime::now(),
			stopping: AtomicBool::new(false),
			reloading: AtomicBool::new(false),
			restarting: AtomicBool::new(false),
			runtime: runtime.clone(),
			signal: broadcast::channel::<&'static str>(1).0,
			log,
			metrics: Metrics::new(runtime),
		}
	}

	pub fn reload(&self) -> Result<()> {
		if cfg!(not(conduit_mods)) {
			return Err!("Reloading not enabled");
		}

		if self.reloading.swap(true, Ordering::AcqRel) {
			return Err!("Reloading already in progress");
		}

		if self.stopping.swap(true, Ordering::AcqRel) {
			return Err!("Shutdown already in progress");
		}

		self.signal("SIGINT").inspect_err(|_| {
			self.stopping.store(false, Ordering::Release);
			self.reloading.store(false, Ordering::Release);
		})
	}

	pub fn restart(&self) -> Result<()> {
		if self.restarting.swap(true, Ordering::AcqRel) {
			return Err!("Restart already in progress");
		}

		self.shutdown()
			.inspect_err(|_| self.restarting.store(false, Ordering::Release))
	}

	pub fn shutdown(&self) -> Result<()> {
		if self.stopping.swap(true, Ordering::AcqRel) {
			return Err!("Shutdown already in progress");
		}

		self.signal("SIGTERM")
			.inspect_err(|_| self.stopping.store(false, Ordering::Release))
	}

	pub fn signal(&self, sig: &'static str) -> Result<()> {
		if let Err(e) = self.signal.send(sig) {
			return Err!("Failed to send signal: {e}");
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
	pub fn check_running(&self) -> Result {
		self.running()
			.then_some(())
			.ok_or_else(|| err!(debug_warn!("Server is shutting down.")))
	}

	#[inline]
	pub fn running(&self) -> bool { !self.stopping.load(Ordering::Acquire) }

	#[inline]
	pub fn is_ours(&self, name: &str) -> bool { name == self.config.server_name }
}
