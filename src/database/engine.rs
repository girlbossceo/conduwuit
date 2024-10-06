use std::{
	collections::{BTreeSet, HashMap},
	ffi::CStr,
	fmt::Write,
	path::PathBuf,
	sync::{atomic::AtomicU32, Arc, Mutex, RwLock},
};

use conduit::{debug, error, info, utils::time::rfc2822_from_seconds, warn, Err, Result, Server};
use rocksdb::{
	backup::{BackupEngine, BackupEngineOptions},
	perf::get_memory_usage_stats,
	AsColumnFamilyRef, BoundColumnFamily, Cache, ColumnFamilyDescriptor, DBCommon, DBWithThreadMode, Env, LogLevel,
	MultiThreaded, Options,
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
	cfs: Mutex<BTreeSet<String>>,
	pub(crate) db: Db,
	corks: AtomicU32,
	pub(super) read_only: bool,
	pub(super) secondary: bool,
}

pub(crate) type Db = DBWithThreadMode<MultiThreaded>;

impl Engine {
	#[tracing::instrument(skip_all)]
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
		let db_opts = db_options(
			config,
			&mut db_env,
			&row_cache,
			col_cache.get("primary").expect("primary cache exists"),
		)?;

		let load_time = std::time::Instant::now();
		if config.rocksdb_repair {
			repair(&db_opts, &config.database_path)?;
		}

		debug!("Listing column families in database");
		let cfs = Db::list_cf(&db_opts, &config.database_path)
			.unwrap_or_default()
			.into_iter()
			.collect::<BTreeSet<_>>();

		debug!("Opening {} column family descriptors in database", cfs.len());
		let cfopts = cfs
			.iter()
			.map(|name| cf_options(config, name, db_opts.clone(), &mut col_cache))
			.collect::<Result<Vec<_>>>()?;

		let cfds = cfs
			.iter()
			.zip(cfopts.into_iter())
			.map(|(name, opts)| ColumnFamilyDescriptor::new(name, opts))
			.collect::<Vec<_>>();

		debug!("Opening database...");
		let path = &config.database_path;
		let res = if config.rocksdb_read_only {
			Db::open_cf_descriptors_read_only(&db_opts, path, cfds, false)
		} else if config.rocksdb_secondary {
			Db::open_cf_descriptors_as_secondary(&db_opts, path, path, cfds)
		} else {
			Db::open_cf_descriptors(&db_opts, path, cfds)
		};

		let db = res.or_else(or_else)?;
		info!(
			columns = cfs.len(),
			sequence = %db.latest_sequence_number(),
			time = ?load_time.elapsed(),
			"Opened database."
		);

		Ok(Arc::new(Self {
			server: server.clone(),
			row_cache,
			col_cache: RwLock::new(col_cache),
			opts: db_opts,
			env: db_env,
			cfs: Mutex::new(cfs),
			db,
			corks: AtomicU32::new(0),
			read_only: config.rocksdb_read_only,
			secondary: config.rocksdb_secondary,
		}))
	}

	#[tracing::instrument(skip(self), level = "trace")]
	pub(crate) fn open_cf(&self, name: &str) -> Result<Arc<BoundColumnFamily<'_>>> {
		let mut cfs = self.cfs.lock().expect("locked");
		if !cfs.contains(name) {
			debug!("Creating new column family in database: {name}");

			let mut col_cache = self.col_cache.write().expect("locked");
			let opts = cf_options(&self.server.config, name, self.opts.clone(), &mut col_cache)?;
			if let Err(e) = self.db.create_cf(name, &opts) {
				error!(?name, "Failed to create new column family: {e}");
				return or_else(e);
			}

			cfs.insert(name.to_owned());
		}

		Ok(self.cf(name))
	}

	pub(crate) fn cf(&self, name: &str) -> Arc<BoundColumnFamily<'_>> {
		self.db
			.cf_handle(name)
			.expect("column was created and exists")
	}

	pub fn flush(&self) -> Result<()> { result(DBCommon::flush_wal(&self.db, false)) }

	pub fn sync(&self) -> Result<()> { result(DBCommon::flush_wal(&self.db, true)) }

	#[inline]
	pub fn corked(&self) -> bool { self.corks.load(std::sync::atomic::Ordering::Relaxed) > 0 }

	pub(crate) fn cork(&self) {
		self.corks
			.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
	}

	pub(crate) fn uncork(&self) {
		self.corks
			.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
	}

	pub fn memory_usage(&self) -> Result<String> {
		let mut res = String::new();
		let stats = get_memory_usage_stats(Some(&[&self.db]), Some(&[&self.row_cache])).or_else(or_else)?;
		let mibs = |input| f64::from(u32::try_from(input / 1024).unwrap_or(0)) / 1024.0;
		writeln!(
			res,
			"Memory buffers: {:.2} MiB\nPending write: {:.2} MiB\nTable readers: {:.2} MiB\nRow cache: {:.2} MiB",
			mibs(stats.mem_table_total),
			mibs(stats.mem_table_unflushed),
			mibs(stats.mem_table_readers_total),
			mibs(u64::try_from(self.row_cache.get_usage())?),
		)?;

		for (name, cache) in &*self.col_cache.read().expect("locked") {
			writeln!(res, "{name} cache: {:.2} MiB", mibs(u64::try_from(cache.get_usage())?))?;
		}

		Ok(res)
	}

	#[tracing::instrument(skip(self), level = "debug")]
	pub fn cleanup(&self) -> Result<()> {
		debug!("Running flush_opt");
		let flushoptions = rocksdb::FlushOptions::default();
		result(DBCommon::flush_opt(&self.db, &flushoptions))
	}

	#[tracing::instrument(skip(self))]
	pub fn backup(&self) -> Result<(), Box<dyn std::error::Error>> {
		let config = &self.server.config;
		let path = config.database_backup_path.as_ref();
		if path.is_none() || path.is_some_and(|path| path.as_os_str().is_empty()) {
			return Ok(());
		}

		let options = BackupEngineOptions::new(path.expect("valid database backup path"))?;
		let mut engine = BackupEngine::open(&options, &self.env)?;
		if config.database_backups_to_keep > 0 {
			if let Err(e) = engine.create_new_backup_flush(&self.db, true) {
				return Err(Box::new(e));
			}

			let engine_info = engine.get_backup_info();
			let info = &engine_info.last().expect("backup engine info is not empty");
			info!(
				"Created database backup #{} using {} bytes in {} files",
				info.backup_id, info.size, info.num_files,
			);
		}

		if config.database_backups_to_keep >= 0 {
			let keep = u32::try_from(config.database_backups_to_keep)?;
			if let Err(e) = engine.purge_old_backups(keep.try_into()?) {
				error!("Failed to purge old backup: {e:?}");
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
		let options = BackupEngineOptions::new(path.expect("valid database backup path")).or_else(or_else)?;
		let engine = BackupEngine::open(&options, &self.env).or_else(or_else)?;
		for info in engine.get_backup_info() {
			writeln!(
				res,
				"#{} {}: {} bytes, {} files",
				info.backup_id,
				rfc2822_from_seconds(info.timestamp),
				info.size,
				info.num_files,
			)?;
		}

		Ok(res)
	}

	pub fn file_list(&self) -> Result<String> {
		match self.db.live_files() {
			Err(e) => Ok(String::from(e)),
			Ok(files) => {
				let mut res = String::new();
				writeln!(res, "| lev  | sst  | keys | dels | size | column |")?;
				writeln!(res, "| ---: | :--- | ---: | ---: | ---: | :---   |")?;
				for file in files {
					writeln!(
						res,
						"| {} | {:<13} | {:7}+ | {:4}- | {:9} | {} |",
						file.level, file.name, file.num_entries, file.num_deletions, file.size, file.column_family_name,
					)?;
				}

				Ok(res)
			},
		}
	}

	/// Query for database property by null-terminated name which is expected to
	/// have a result with an integer representation. This is intended for
	/// low-overhead programmatic use.
	pub(crate) fn property_integer(&self, cf: &impl AsColumnFamilyRef, name: &CStr) -> Result<u64> {
		result(self.db.property_int_value_cf(cf, name))
			.and_then(|val| val.map_or_else(|| Err!("Property {name:?} not found."), Ok))
	}

	/// Query for database property by name receiving the result in a string.
	pub(crate) fn property(&self, cf: &impl AsColumnFamilyRef, name: &str) -> Result<String> {
		result(self.db.property_value_cf(cf, name))
			.and_then(|val| val.map_or_else(|| Err!("Property {name:?} not found."), Ok))
	}
}

pub(crate) fn repair(db_opts: &Options, path: &PathBuf) -> Result<()> {
	warn!("Starting database repair. This may take a long time...");
	match Db::repair(db_opts, path) {
		Ok(()) => info!("Database repair successful."),
		Err(e) => return Err!("Repair failed: {e:?}"),
	}

	Ok(())
}

#[tracing::instrument(skip_all, name = "rocksdb")]
pub(crate) fn handle_log(level: LogLevel, msg: &str) {
	let msg = msg.trim();
	if msg.starts_with("Options") {
		return;
	}

	match level {
		LogLevel::Header | LogLevel::Debug => debug!("{msg}"),
		LogLevel::Error | LogLevel::Fatal => error!("{msg}"),
		LogLevel::Info => debug!("{msg}"),
		LogLevel::Warn => warn!("{msg}"),
	};
}

impl Drop for Engine {
	#[cold]
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

		info!(
			sequence = %self.db.latest_sequence_number(),
			"Closing database..."
		);
	}
}
