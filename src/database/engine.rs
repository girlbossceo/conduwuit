mod backup;
mod cf_opts;
pub(crate) mod context;
mod db_opts;
pub(crate) mod descriptor;
mod files;
mod logger;
mod memory_usage;
mod open;
mod repair;

use std::{
	ffi::CStr,
	sync::{
		atomic::{AtomicU32, Ordering},
		Arc,
	},
};

use conduwuit::{debug, info, warn, Err, Result};
use rocksdb::{
	AsColumnFamilyRef, BoundColumnFamily, DBCommon, DBWithThreadMode, MultiThreaded,
	WaitForCompactOptions,
};

use crate::{
	pool::Pool,
	util::{map_err, result},
	Context,
};

pub struct Engine {
	pub(super) read_only: bool,
	pub(super) secondary: bool,
	corks: AtomicU32,
	pub(crate) db: Db,
	pub(crate) pool: Arc<Pool>,
	pub(crate) ctx: Arc<Context>,
}

pub(crate) type Db = DBWithThreadMode<MultiThreaded>;

impl Engine {
	pub(crate) fn cf(&self, name: &str) -> Arc<BoundColumnFamily<'_>> {
		self.db
			.cf_handle(name)
			.expect("column must be described prior to database open")
	}

	#[inline]
	pub(crate) fn cork(&self) { self.corks.fetch_add(1, Ordering::Relaxed); }

	#[inline]
	pub(crate) fn uncork(&self) { self.corks.fetch_sub(1, Ordering::Relaxed); }

	#[inline]
	pub fn corked(&self) -> bool { self.corks.load(Ordering::Relaxed) > 0 }

	#[tracing::instrument(skip(self))]
	pub fn sync(&self) -> Result { result(DBCommon::flush_wal(&self.db, true)) }

	#[tracing::instrument(skip(self), level = "debug")]
	pub fn flush(&self) -> Result { result(DBCommon::flush_wal(&self.db, false)) }

	#[tracing::instrument(skip(self), level = "info")]
	pub fn sort(&self) -> Result {
		let flushoptions = rocksdb::FlushOptions::default();
		result(DBCommon::flush_opt(&self.db, &flushoptions))
	}

	#[tracing::instrument(skip(self), level = "info")]
	pub fn wait_compactions(&self) -> Result {
		let mut opts = WaitForCompactOptions::default();
		opts.set_abort_on_pause(true);
		opts.set_flush(false);
		opts.set_timeout(0);

		self.db.wait_for_compact(&opts).map_err(map_err)
	}

	/// Query for database property by null-terminated name which is expected to
	/// have a result with an integer representation. This is intended for
	/// low-overhead programmatic use.
	pub(crate) fn property_integer(
		&self,
		cf: &impl AsColumnFamilyRef,
		name: &CStr,
	) -> Result<u64> {
		result(self.db.property_int_value_cf(cf, name))
			.and_then(|val| val.map_or_else(|| Err!("Property {name:?} not found."), Ok))
	}

	/// Query for database property by name receiving the result in a string.
	pub(crate) fn property(&self, cf: &impl AsColumnFamilyRef, name: &str) -> Result<String> {
		result(self.db.property_value_cf(cf, name))
			.and_then(|val| val.map_or_else(|| Err!("Property {name:?} not found."), Ok))
	}

	#[inline]
	#[must_use]
	pub fn is_read_only(&self) -> bool { self.secondary || self.read_only }

	#[inline]
	#[must_use]
	pub fn is_secondary(&self) -> bool { self.secondary }
}

impl Drop for Engine {
	#[cold]
	fn drop(&mut self) {
		const BLOCKING: bool = true;

		debug!("Waiting for background tasks to finish...");
		self.db.cancel_all_background_work(BLOCKING);

		info!(
			sequence = %self.db.latest_sequence_number(),
			"Closing database..."
		);
	}
}
