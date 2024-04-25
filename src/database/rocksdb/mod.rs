use std::{
	collections::HashMap,
	sync::{atomic::AtomicU32, Arc},
};

use chrono::{DateTime, Utc};
use rust_rocksdb::{
	backup::{BackupEngine, BackupEngineOptions},
	Cache, ColumnFamilyDescriptor, DBCommon, DBWithThreadMode as Db, Env, MultiThreaded, Options,
};
use tracing::{debug, error, info, warn};

use super::{super::Config, watchers::Watchers, KeyValueDatabaseEngine, KvTree};
use crate::Result;

pub(crate) mod kvtree;
pub(crate) mod opts;

use kvtree::RocksDbEngineTree;
use opts::{cf_options, db_options};

use super::watchers;

pub(crate) struct Engine {
	rocks: Db<MultiThreaded>,
	row_cache: Cache,
	col_cache: HashMap<String, Cache>,
	old_cfs: Vec<String>,
	opts: Options,
	env: Env,
	config: Config,
	corks: AtomicU32,
}

impl KeyValueDatabaseEngine for Arc<Engine> {
	fn open(config: &Config) -> Result<Self> {
		let cache_capacity_bytes = config.db_cache_capacity_mb * 1024.0 * 1024.0;
		let row_cache_capacity_bytes = (cache_capacity_bytes * 0.50) as usize;
		let col_cache_capacity_bytes = (cache_capacity_bytes * 0.50) as usize;

		let mut col_cache = HashMap::new();
		col_cache.insert("primary".to_owned(), Cache::new_lru_cache(col_cache_capacity_bytes));

		let mut db_env = Env::new()?;
		let row_cache = Cache::new_lru_cache(row_cache_capacity_bytes);
		let db_opts = db_options(config, &mut db_env, &row_cache, col_cache.get("primary").expect("cache"));

		let load_time = std::time::Instant::now();
		if config.rocksdb_repair {
			warn!("Starting database repair. This may take a long time...");
			if let Err(e) = Db::<MultiThreaded>::repair(&db_opts, &config.database_path) {
				error!("Repair failed: {:?}", e);
			}
		}

		debug!("Listing column families in database");
		let cfs = Db::<MultiThreaded>::list_cf(&db_opts, &config.database_path).unwrap_or_default();

		debug!("Opening {} column family descriptors in database", cfs.len());
		let cfds = cfs
			.iter()
			.map(|name| ColumnFamilyDescriptor::new(name, cf_options(config, name, db_opts.clone(), &mut col_cache)))
			.collect::<Vec<_>>();

		debug!("Opening database...");
		let db = if config.rocksdb_read_only {
			Db::<MultiThreaded>::open_cf_for_read_only(&db_opts, &config.database_path, cfs.clone(), false)?
		} else {
			Db::<MultiThreaded>::open_cf_descriptors(&db_opts, &config.database_path, cfds)?
		};

		info!(
			"Opened database at sequence number {} in {:?}",
			db.latest_sequence_number(),
			load_time.elapsed()
		);
		Ok(Arc::new(Engine {
			rocks: db,
			row_cache,
			col_cache,
			old_cfs: cfs,
			opts: db_opts,
			env: db_env,
			config: config.clone(),
			corks: AtomicU32::new(0),
		}))
	}

	fn open_tree(&self, name: &'static str) -> Result<Arc<dyn KvTree>> {
		if !self.old_cfs.contains(&name.to_owned()) {
			// Create if it didn't exist
			debug!("Creating new column family in database: {}", name);
			_ = self.rocks.create_cf(name, &self.opts);
		}

		Ok(Arc::new(RocksDbEngineTree {
			name,
			db: Arc::clone(self),
			watchers: Watchers::default(),
		}))
	}

	fn flush(&self) -> Result<()> {
		DBCommon::flush_wal(&self.rocks, false)?;

		Ok(())
	}

	fn sync(&self) -> Result<()> {
		DBCommon::flush_wal(&self.rocks, true)?;

		Ok(())
	}

	fn corked(&self) -> bool { self.corks.load(std::sync::atomic::Ordering::Relaxed) > 0 }

	fn cork(&self) -> Result<()> {
		self.corks
			.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

		Ok(())
	}

	fn uncork(&self) -> Result<()> {
		self.corks
			.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);

		Ok(())
	}

	fn memory_usage(&self) -> Result<String> {
		let mut res = String::new();
		let stats = rust_rocksdb::perf::get_memory_usage_stats(Some(&[&self.rocks]), Some(&[&self.row_cache]))?;
		_ = std::fmt::write(
			&mut res,
			format_args!(
				"Memory buffers: {:.2} MiB\nPending write: {:.2} MiB\nTable readers: {:.2} MiB\nRow cache: {:.2} MiB\n",
				stats.mem_table_total as f64 / 1024.0 / 1024.0,
				stats.mem_table_unflushed as f64 / 1024.0 / 1024.0,
				stats.mem_table_readers_total as f64 / 1024.0 / 1024.0,
				self.row_cache.get_usage() as f64 / 1024.0 / 1024.0,
			),
		);

		for (name, cache) in &self.col_cache {
			_ = std::fmt::write(
				&mut res,
				format_args!("{} cache: {:.2} MiB\n", name, cache.get_usage() as f64 / 1024.0 / 1024.0,),
			);
		}

		Ok(res)
	}

	fn cleanup(&self) -> Result<()> {
		debug!("Running flush_opt");
		let flushoptions = rust_rocksdb::FlushOptions::default();

		DBCommon::flush_opt(&self.rocks, &flushoptions)?;

		Ok(())
	}

	fn backup(&self) -> Result<(), Box<dyn std::error::Error>> {
		let path = self.config.database_backup_path.as_ref();
		if path.is_none() || path.is_some_and(|path| path.as_os_str().is_empty()) {
			return Ok(());
		}

		let options = BackupEngineOptions::new(path.unwrap())?;
		let mut engine = BackupEngine::open(&options, &self.env)?;
		let ret = if self.config.database_backups_to_keep > 0 {
			if let Err(e) = engine.create_new_backup_flush(&self.rocks, true) {
				return Err(Box::new(e));
			}

			let engine_info = engine.get_backup_info();
			let info = &engine_info.last().unwrap();
			info!(
				"Created database backup #{} using {} bytes in {} files",
				info.backup_id, info.size, info.num_files,
			);
			Ok(())
		} else {
			Ok(())
		};

		if self.config.database_backups_to_keep >= 0 {
			let keep = u32::try_from(self.config.database_backups_to_keep)?;
			if let Err(e) = engine.purge_old_backups(keep.try_into()?) {
				error!("Failed to purge old backup: {:?}", e.to_string());
			}
		}

		ret
	}

	fn backup_list(&self) -> Result<String> {
		let path = self.config.database_backup_path.as_ref();
		if path.is_none() || path.is_some_and(|path| path.as_os_str().is_empty()) {
			return Ok(
				"Configure database_backup_path to enable backups, or the path specified is not valid".to_owned(),
			);
		}

		let mut res = String::new();
		let options = BackupEngineOptions::new(path.unwrap())?;
		let engine = BackupEngine::open(&options, &self.env)?;
		for info in engine.get_backup_info() {
			std::fmt::write(
				&mut res,
				format_args!(
					"#{} {}: {} bytes, {} files\n",
					info.backup_id,
					DateTime::<Utc>::from_timestamp(info.timestamp, 0)
						.unwrap_or_default()
						.to_rfc2822(),
					info.size,
					info.num_files,
				),
			)
			.unwrap();
		}

		Ok(res)
	}

	fn file_list(&self) -> Result<String> {
		match self.rocks.live_files() {
			Err(e) => Ok(String::from(e)),
			Ok(files) => {
				let mut res = String::new();
				for file in files {
					let _ = std::fmt::write(
						&mut res,
						format_args!(
							"<code>L{} {:<13} {:7}+ {:4}- {:9}</code> {}<br>",
							file.level,
							file.name,
							file.num_entries,
							file.num_deletions,
							file.size,
							file.column_family_name,
						),
					);
				}
				Ok(res)
			},
		}
	}

	// TODO: figure out if this is needed for rocksdb
	#[allow(dead_code)]
	fn clear_caches(&self) {}
}
