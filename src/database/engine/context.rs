use std::{
	collections::BTreeMap,
	sync::{Arc, Mutex},
};

use conduwuit::{debug, utils::math::usize_from_f64, Result, Server};
use rocksdb::{Cache, Env};

use crate::{or_else, pool::Pool};

/// Some components are constructed prior to opening the database and must
/// outlive the database. These can also be shared between database instances
/// though at the time of this comment we only open one database per process.
/// These assets are housed in the shared Context.
pub(crate) struct Context {
	pub(crate) pool: Arc<Pool>,
	pub(crate) col_cache: Mutex<BTreeMap<String, Cache>>,
	pub(crate) row_cache: Mutex<Cache>,
	pub(crate) env: Mutex<Env>,
	pub(crate) server: Arc<Server>,
}

impl Context {
	pub(crate) fn new(server: &Arc<Server>) -> Result<Arc<Self>> {
		let config = &server.config;
		let cache_capacity_bytes = config.db_cache_capacity_mb * 1024.0 * 1024.0;

		let row_cache_capacity_bytes = usize_from_f64(cache_capacity_bytes * 0.50)?;
		let row_cache = Cache::new_lru_cache(row_cache_capacity_bytes);

		let col_cache_capacity_bytes = usize_from_f64(cache_capacity_bytes * 0.50)?;
		let col_cache = Cache::new_lru_cache(col_cache_capacity_bytes);

		let col_cache: BTreeMap<_, _> = [("Shared".to_owned(), col_cache)].into();

		let mut env = Env::new().or_else(or_else)?;

		if config.rocksdb_compaction_prio_idle {
			env.lower_thread_pool_cpu_priority();
		}

		if config.rocksdb_compaction_ioprio_idle {
			env.lower_thread_pool_io_priority();
		}

		Ok(Arc::new(Self {
			pool: Pool::new(server)?,
			col_cache: col_cache.into(),
			row_cache: row_cache.into(),
			env: env.into(),
			server: server.clone(),
		}))
	}
}

impl Drop for Context {
	#[cold]
	fn drop(&mut self) {
		debug!("Closing frontend pool");
		self.pool.close();

		let mut env = self.env.lock().expect("locked");

		debug!("Shutting down background threads");
		env.set_high_priority_background_threads(0);
		env.set_low_priority_background_threads(0);
		env.set_bottom_priority_background_threads(0);
		env.set_background_threads(0);

		debug!("Joining background threads...");
		env.join_all_threads();
	}
}
