use std::{
	collections::{HashMap, HashSet},
	fmt::Write,
	sync::{atomic::AtomicU32, Arc, Mutex, RwLock},
};

use chrono::{DateTime, Utc};
use conduit::{debug, error, info, warn, Result, Server};
use rocksdb::{
	backup::{BackupEngine, BackupEngineOptions},
	perf::get_memory_usage_stats,
	BoundColumnFamily, Cache, ColumnFamilyDescriptor, DBCommon, DBWithThreadMode, Env, MultiThreaded, Options,
};

use crate::{
	opts::{cf_options, db_options},
	or_else, result,
};

pub struct Engine {
	server: Arc<Server>,
	row_cache: Cache,
	col_cache: RwLock<HashMap<String, Cache>>,
	opts: Options,
	env: Env,
	cfs: Mutex<HashSet<String>>,
	pub(crate) db: Db,
	corks: AtomicU32,
}

pub(crate) type Db = DBWithThreadMode<MultiThreaded>;

impl Engine {
	pub(crate) fn open(server: &Arc<Server>) -> Result<Arc<Self>> {
		let config = &server.config;
		let cache_capacity_bytes = config.db_cache_capacity_mb * 1024.0 * 1024.0;

		#[allow(clippy::as_conversions, clippy::cast_sign_loss, clippy::cast_possible_truncation)]
		let row_cache_capacity_bytes = (cache_capacity_bytes * 0.50) as usize;

		#[allow(clippy::as_conversions, clippy::cast_sign_loss, clippy::cast_possible_truncation)]
		let col_cache_capacity_bytes = (cache_capacity_bytes * 0.50) as usize;

		let mut col_cache = HashMap::new();
		col_cache.insert("primary".to_owned(), Cache::new_lru_cache(col_cache_capacity_bytes));

		let mut db_env = Env::new().or_else(or_else)?;
		let row_cache = Cache::new_lru_cache(row_cache_capacity_bytes);
		let db_opts = db_options(config, &mut db_env, &row_cache, col_cache.get("primary").expect("cache"));

		let load_time = std::time::Instant::now();
		if config.rocksdb_repair {
			warn!("Starting database repair. This may take a long time...");
			if let Err(e) = Db::repair(&db_opts, &config.database_path) {
				error!("Repair failed: {:?}", e);
			}
		}

		debug!("Listing column families in database");
		let cfs = Db::list_cf(&db_opts, &config.database_path).unwrap_or_default();

		debug!("Opening {} column family descriptors in database", cfs.len());
		let cfds = cfs
			.iter()
			.map(|name| ColumnFamilyDescriptor::new(name, cf_options(config, name, db_opts.clone(), &mut col_cache)))
			.collect::<Vec<_>>();

		debug!("Opening database...");
		let res = if config.rocksdb_read_only {
			Db::open_cf_for_read_only(&db_opts, &config.database_path, cfs.clone(), false)
		} else {
			Db::open_cf_descriptors(&db_opts, &config.database_path, cfds)
		};

		let db = res.or_else(or_else)?;
		info!(
			"Opened database at sequence number {} in {:?}",
			db.latest_sequence_number(),
			load_time.elapsed()
		);

		let cfs = HashSet::<String>::from_iter(cfs);
		Ok(Arc::new(Self {
			server: server.clone(),
			row_cache,
			col_cache: RwLock::new(col_cache),
			opts: db_opts,
			env: db_env,
			cfs: Mutex::new(cfs),
			db,
			corks: AtomicU32::new(0),
		}))
	}

	pub(crate) fn open_cf(&self, name: &str) -> Result<Arc<BoundColumnFamily<'_>>> {
		let mut cfs = self.cfs.lock().expect("locked");
		if !cfs.contains(name) {
			debug!("Creating new column family in database: {}", name);

			let mut col_cache = self.col_cache.write().expect("locked");
			let opts = cf_options(&self.server.config, name, self.opts.clone(), &mut col_cache);
			if let Err(e) = self.db.create_cf(name, &opts) {
				error!("Failed to create new column family: {e}");
				return or_else(e);
			}

			cfs.insert(name.to_owned());
		}

		Ok(self.cf(name))
	}

	pub(crate) fn cf<'db>(&'db self, name: &str) -> Arc<BoundColumnFamily<'db>> {
		self.db
			.cf_handle(name)
			.expect("column was created and exists")
	}

	pub fn flush(&self) -> Result<()> { result(DBCommon::flush_wal(&self.db, false)) }

	pub fn sync(&self) -> Result<()> { result(DBCommon::flush_wal(&self.db, true)) }

	pub(crate) fn corked(&self) -> bool { self.corks.load(std::sync::atomic::Ordering::Relaxed) > 0 }

	pub(crate) fn cork(&self) {
		self.corks
			.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
	}

	pub(crate) fn uncork(&self) {
		self.corks
			.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
	}

	#[allow(clippy::as_conversions, clippy::cast_sign_loss, clippy::cast_possible_truncation)]
	pub fn memory_usage(&self) -> Result<String> {
		let mut res = String::new();
		let stats = get_memory_usage_stats(Some(&[&self.db]), Some(&[&self.row_cache])).or_else(or_else)?;
		writeln!(
			res,
			"Memory buffers: {:.2} MiB\nPending write: {:.2} MiB\nTable readers: {:.2} MiB\nRow cache: {:.2} MiB",
			stats.mem_table_total as f64 / 1024.0 / 1024.0,
			stats.mem_table_unflushed as f64 / 1024.0 / 1024.0,
			stats.mem_table_readers_total as f64 / 1024.0 / 1024.0,
			self.row_cache.get_usage() as f64 / 1024.0 / 1024.0,
		)
		.expect("should be able to write to string buffer");

		for (name, cache) in &*self.col_cache.read().expect("locked") {
			writeln!(res, "{} cache: {:.2} MiB", name, cache.get_usage() as f64 / 1024.0 / 1024.0,)
				.expect("should be able to write to string buffer");
		}

		Ok(res)
	}

	pub fn cleanup(&self) -> Result<()> {
		debug!("Running flush_opt");
		let flushoptions = rocksdb::FlushOptions::default();
		result(DBCommon::flush_opt(&self.db, &flushoptions))
	}

	pub fn backup(&self) -> Result<(), Box<dyn std::error::Error>> {
		let config = &self.server.config;
		let path = config.database_backup_path.as_ref();
		if path.is_none() || path.is_some_and(|path| path.as_os_str().is_empty()) {
			return Ok(());
		}

		let options = BackupEngineOptions::new(path.unwrap())?;
		let mut engine = BackupEngine::open(&options, &self.env)?;
		if config.database_backups_to_keep > 0 {
			if let Err(e) = engine.create_new_backup_flush(&self.db, true) {
				return Err(Box::new(e));
			}

			let engine_info = engine.get_backup_info();
			let info = &engine_info.last().unwrap();
			info!(
				"Created database backup #{} using {} bytes in {} files",
				info.backup_id, info.size, info.num_files,
			);
		}

		if config.database_backups_to_keep >= 0 {
			let keep = u32::try_from(config.database_backups_to_keep)?;
			if let Err(e) = engine.purge_old_backups(keep.try_into()?) {
				error!("Failed to purge old backup: {:?}", e.to_string());
			}
		}

		Ok(())
	}

	pub fn backup_list(&self) -> Result<String> {
		let config = &self.server.config;
		let path = config.database_backup_path.as_ref();
		if path.is_none() || path.is_some_and(|path| path.as_os_str().is_empty()) {
			return Ok(
				"Configure database_backup_path to enable backups, or the path specified is not valid".to_owned(),
			);
		}

		let mut res = String::new();
		let options = BackupEngineOptions::new(path.expect("valid path")).or_else(or_else)?;
		let engine = BackupEngine::open(&options, &self.env).or_else(or_else)?;
		for info in engine.get_backup_info() {
			writeln!(
				res,
				"#{} {}: {} bytes, {} files",
				info.backup_id,
				DateTime::<Utc>::from_timestamp(info.timestamp, 0)
					.unwrap_or_default()
					.to_rfc2822(),
				info.size,
				info.num_files,
			)
			.expect("should be able to write to string buffer");
		}

		Ok(res)
	}

	pub fn file_list(&self) -> Result<String> {
		match self.db.live_files() {
			Err(e) => Ok(String::from(e)),
			Ok(files) => {
				let mut res = String::new();
				writeln!(res, "| lev  | sst  | keys | dels | size | column |").expect("written to string buffer");
				writeln!(res, "| ---: | :--- | ---: | ---: | ---: | :---   |").expect("written to string buffer");
				for file in files {
					writeln!(
						res,
						"| {} | {:<13} | {:7}+ | {:4}- | {:9} | {} |",
						file.level, file.name, file.num_entries, file.num_deletions, file.size, file.column_family_name,
					)
					.expect("should be able to writeln to string buffer");
				}
				Ok(res)
			},
		}
	}
}

impl Drop for Engine {
	fn drop(&mut self) {
		const BLOCKING: bool = true;

		debug!("Waiting for background tasks to finish...");
		self.db.cancel_all_background_work(BLOCKING);

		debug!("Shutting down background threads");
		self.env.set_high_priority_background_threads(0);
		self.env.set_low_priority_background_threads(0);
		self.env.set_bottom_priority_background_threads(0);
		self.env.set_background_threads(0);

		debug!("Joining background threads...");
		self.env.join_all_threads();
	}
}
