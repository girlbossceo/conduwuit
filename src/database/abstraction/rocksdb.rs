use std::{
	future::Future,
	pin::Pin,
	sync::{Arc, RwLock},
};

use chrono::{DateTime, Utc};
use rust_rocksdb::{
	backup::{BackupEngine, BackupEngineOptions},
	LogLevel::{Debug, Error, Fatal, Info, Warn},
	WriteBatchWithTransaction,
};
use tracing::{debug, error, info};

use super::{super::Config, watchers::Watchers, KeyValueDatabaseEngine, KvTree};
use crate::{utils, Result};

pub(crate) struct Engine {
	rocks: rust_rocksdb::DBWithThreadMode<rust_rocksdb::MultiThreaded>,
	row_cache: rust_rocksdb::Cache,
	col_cache: rust_rocksdb::Cache,
	old_cfs: Vec<String>,
	opts: rust_rocksdb::Options,
	env: rust_rocksdb::Env,
	config: Config,
}

struct RocksDbEngineTree<'a> {
	db: Arc<Engine>,
	name: &'a str,
	watchers: Watchers,
	write_lock: RwLock<()>,
}

fn db_options(
	config: &Config, env: &rust_rocksdb::Env, row_cache: &rust_rocksdb::Cache, col_cache: &rust_rocksdb::Cache,
) -> rust_rocksdb::Options {
	// database options: https://docs.rs/rocksdb/latest/rocksdb/struct.Options.html#
	let mut db_opts = rust_rocksdb::Options::default();

	// Logging
	let rocksdb_log_level = match config.rocksdb_log_level.as_ref() {
		"debug" => Debug,
		"info" => Info,
		"warn" => Warn,
		"fatal" => Fatal,
		_ => Error,
	};
	db_opts.set_log_level(rocksdb_log_level);
	db_opts.set_max_log_file_size(config.rocksdb_max_log_file_size);
	db_opts.set_log_file_time_to_roll(config.rocksdb_log_time_to_roll);
	db_opts.set_keep_log_file_num(config.rocksdb_max_log_files);

	// Processing
	let threads = if config.rocksdb_parallelism_threads == 0 {
		num_cpus::get_physical() // max cores if user specified 0
	} else {
		config.rocksdb_parallelism_threads
	};

	db_opts.set_max_background_jobs(threads.try_into().unwrap());
	db_opts.set_max_subcompactions(threads.try_into().unwrap());

	// IO
	db_opts.set_use_direct_reads(true);
	db_opts.set_use_direct_io_for_flush_and_compaction(true);
	if config.rocksdb_optimize_for_spinning_disks {
		db_opts.set_skip_stats_update_on_db_open(true); // speeds up opening DB on hard
		                                        // drives
	}

	// Blocks
	let mut block_based_options = rust_rocksdb::BlockBasedOptions::default();
	block_based_options.set_block_size(4 * 1024);
	block_based_options.set_metadata_block_size(4 * 1024);
	block_based_options.set_bloom_filter(9.6, true);
	block_based_options.set_optimize_filters_for_memory(true);
	block_based_options.set_cache_index_and_filter_blocks(true);
	block_based_options.set_pin_top_level_index_and_filter(true);
	block_based_options.set_block_cache(col_cache);
	db_opts.set_row_cache(row_cache);

	// Buffers
	db_opts.set_write_buffer_size(2 * 1024 * 1024);
	db_opts.set_max_write_buffer_number(2);
	db_opts.set_min_write_buffer_number(1);

	// Files
	db_opts.set_level_zero_file_num_compaction_trigger(1);
	db_opts.set_target_file_size_base(64 * 1024 * 1024);
	db_opts.set_max_bytes_for_level_base(128 * 1024 * 1024);
	db_opts.set_ttl(14 * 24 * 60 * 60);

	// Compression
	let rocksdb_compression_algo = match config.rocksdb_compression_algo.as_ref() {
		"zstd" => rust_rocksdb::DBCompressionType::Zstd,
		"zlib" => rust_rocksdb::DBCompressionType::Zlib,
		"lz4" => rust_rocksdb::DBCompressionType::Lz4,
		"bz2" => rust_rocksdb::DBCompressionType::Bz2,
		_ => rust_rocksdb::DBCompressionType::Zstd,
	};

	if config.rocksdb_bottommost_compression {
		db_opts.set_bottommost_compression_type(rocksdb_compression_algo);
		db_opts.set_bottommost_zstd_max_train_bytes(0, true);

		// -14 w_bits is only read by zlib.
		db_opts.set_bottommost_compression_options(-14, config.rocksdb_bottommost_compression_level, 0, 0, true);
	}

	// -14 w_bits is only read by zlib.
	db_opts.set_compression_options(-14, config.rocksdb_compression_level, 0, 0);
	db_opts.set_compression_type(rocksdb_compression_algo);

	// Misc
	db_opts.create_if_missing(true);

	// https://github.com/facebook/rocksdb/wiki/WAL-Recovery-Modes#ktoleratecorruptedtailrecords
	//
	// Unclean shutdowns of a Matrix homeserver are likely to be fine when
	// recovered in this manner as it's likely any lost information will be
	// restored via federation.
	db_opts.set_wal_recovery_mode(rust_rocksdb::DBRecoveryMode::TolerateCorruptedTailRecords);

	// TODO: remove me? https://gitlab.com/famedly/conduit/-/merge_requests/602/diffs#a3a261d6a9014330581b5bdecd586dab5ae00245_62_54
	let prefix_extractor = rust_rocksdb::SliceTransform::create_fixed_prefix(1);
	db_opts.set_prefix_extractor(prefix_extractor);

	db_opts.set_block_based_table_factory(&block_based_options);
	db_opts.set_env(env);
	db_opts
}

impl KeyValueDatabaseEngine for Arc<Engine> {
	fn open(config: &Config) -> Result<Self> {
		let cache_capacity_bytes = config.db_cache_capacity_mb * 1024.0 * 1024.0;
		let row_cache_capacity_bytes = (cache_capacity_bytes * 0.25) as usize;
		let col_cache_capacity_bytes = (cache_capacity_bytes * 0.75) as usize;

		let db_env = rust_rocksdb::Env::new()?;
		let row_cache = rust_rocksdb::Cache::new_lru_cache(row_cache_capacity_bytes);
		let col_cache = rust_rocksdb::Cache::new_lru_cache(col_cache_capacity_bytes);
		let db_opts = db_options(config, &db_env, &row_cache, &col_cache);

		debug!("Listing column families in database");
		let cfs =
			rust_rocksdb::DBWithThreadMode::<rust_rocksdb::MultiThreaded>::list_cf(&db_opts, &config.database_path)
				.unwrap_or_default();

		debug!("Opening column family descriptors in database");
		info!("RocksDB database compaction will take place now, a delay in startup is expected");
		let db = rust_rocksdb::DBWithThreadMode::<rust_rocksdb::MultiThreaded>::open_cf_descriptors(
			&db_opts,
			&config.database_path,
			cfs.iter().map(|name| rust_rocksdb::ColumnFamilyDescriptor::new(name, db_opts.clone())),
		)?;

		Ok(Arc::new(Engine {
			rocks: db,
			row_cache,
			col_cache,
			old_cfs: cfs,
			opts: db_opts,
			env: db_env,
			config: config.clone(),
		}))
	}

	fn open_tree(&self, name: &'static str) -> Result<Arc<dyn KvTree>> {
		if !self.old_cfs.contains(&name.to_owned()) {
			// Create if it didn't exist
			debug!("Creating new column family in database: {}", name);
			let _ = self.rocks.create_cf(name, &self.opts);
		}

		Ok(Arc::new(RocksDbEngineTree {
			name,
			db: Arc::clone(self),
			watchers: Watchers::default(),
			write_lock: RwLock::new(()),
		}))
	}

	fn flush(&self) -> Result<()> {
		rust_rocksdb::DBCommon::flush_wal(&self.rocks, false)?;

		Ok(())
	}

	fn sync(&self) -> Result<()> {
		rust_rocksdb::DBCommon::flush_wal(&self.rocks, true)?;

		Ok(())
	}

	fn memory_usage(&self) -> Result<String> {
		let stats = rust_rocksdb::perf::get_memory_usage_stats(
			Some(&[&self.rocks]),
			Some(&[&self.row_cache, &self.col_cache]),
		)?;
		Ok(format!(
			"Approximate memory usage of all the mem-tables: {:.3} MB\nApproximate memory usage of un-flushed \
			 mem-tables: {:.3} MB\nApproximate memory usage of all the table readers: {:.3} MB\nApproximate memory \
			 usage by cache: {:.3} MB\nApproximate memory usage by row cache: {:.3} MB pinned: {:.3} MB\nApproximate \
			 memory usage by column cache: {:.3} MB pinned: {:.3} MB\n",
			stats.mem_table_total as f64 / 1024.0 / 1024.0,
			stats.mem_table_unflushed as f64 / 1024.0 / 1024.0,
			stats.mem_table_readers_total as f64 / 1024.0 / 1024.0,
			stats.cache_total as f64 / 1024.0 / 1024.0,
			self.row_cache.get_usage() as f64 / 1024.0 / 1024.0,
			self.row_cache.get_pinned_usage() as f64 / 1024.0 / 1024.0,
			self.col_cache.get_usage() as f64 / 1024.0 / 1024.0,
			self.col_cache.get_pinned_usage() as f64 / 1024.0 / 1024.0,
		))
	}

	fn cleanup(&self) -> Result<()> {
		debug!("Running flush_opt");
		let flushoptions = rust_rocksdb::FlushOptions::default();

		rust_rocksdb::DBCommon::flush_opt(&self.rocks, &flushoptions)?;

		Ok(())
	}

	fn backup(&self) -> Result<(), Box<dyn std::error::Error>> {
		let path = self.config.database_backup_path.as_ref();
		if path.is_none() || path.is_some_and(String::is_empty) {
			return Ok(());
		}

		let options = BackupEngineOptions::new(path.unwrap())?;
		let mut engine = BackupEngine::open(&options, &self.env)?;
		let ret = if self.config.database_backups_to_keep > 0 {
			match engine.create_new_backup_flush(&self.rocks, true) {
				Err(e) => return Err(Box::new(e)),
				Ok(_) => {
					let _info = engine.get_backup_info();
					let info = &_info.last().unwrap();
					info!(
						"Created database backup #{} using {} bytes in {} files",
						info.backup_id, info.size, info.num_files,
					);
					Ok(())
				},
			}
		} else {
			Ok(())
		};

		if self.config.database_backups_to_keep >= 0 {
			let keep = u32::try_from(self.config.database_backups_to_keep)?;
			if let Err(e) = engine.purge_old_backups(keep.try_into()?) {
				error!("Failed to purge old backup: {:?}", e.to_string())
			}
		}

		ret
	}

	fn backup_list(&self) -> Result<String> {
		let path = self.config.database_backup_path.as_ref();
		if path.is_none() || path.is_some_and(String::is_empty) {
			return Ok("Configure database_backup_path to enable backups".to_owned());
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
					DateTime::<Utc>::from_timestamp(info.timestamp, 0).unwrap().to_rfc2822(),
					info.size,
					info.num_files,
				),
			)
			.unwrap();
		}

		Ok(res)
	}

	// TODO: figure out if this is needed for rocksdb
	#[allow(dead_code)]
	fn clear_caches(&self) {}
}

impl RocksDbEngineTree<'_> {
	fn cf(&self) -> Arc<rust_rocksdb::BoundColumnFamily<'_>> { self.db.rocks.cf_handle(self.name).unwrap() }
}

impl KvTree for RocksDbEngineTree<'_> {
	fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
		let mut readoptions = rust_rocksdb::ReadOptions::default();
		readoptions.set_total_order_seek(true);

		Ok(self.db.rocks.get_cf_opt(&self.cf(), key, &readoptions)?)
	}

	fn multi_get(
		&self, iter: Vec<(&Arc<rust_rocksdb::BoundColumnFamily<'_>>, Vec<u8>)>,
	) -> Vec<std::result::Result<Option<Vec<u8>>, rust_rocksdb::Error>> {
		let mut readoptions = rust_rocksdb::ReadOptions::default();
		readoptions.set_total_order_seek(true);

		self.db.rocks.multi_get_cf_opt(iter, &readoptions)
	}

	fn insert(&self, key: &[u8], value: &[u8]) -> Result<()> {
		let writeoptions = rust_rocksdb::WriteOptions::default();
		let lock = self.write_lock.read().unwrap();

		self.db.rocks.put_cf_opt(&self.cf(), key, value, &writeoptions)?;

		drop(lock);

		self.watchers.wake(key);

		Ok(())
	}

	fn insert_batch(&self, iter: &mut dyn Iterator<Item = (Vec<u8>, Vec<u8>)>) -> Result<()> {
		let writeoptions = rust_rocksdb::WriteOptions::default();

		let mut batch = WriteBatchWithTransaction::<false>::default();

		for (key, value) in iter {
			batch.put_cf(&self.cf(), key, value);
		}

		Ok(self.db.rocks.write_opt(batch, &writeoptions)?)
	}

	fn remove(&self, key: &[u8]) -> Result<()> {
		let writeoptions = rust_rocksdb::WriteOptions::default();

		Ok(self.db.rocks.delete_cf_opt(&self.cf(), key, &writeoptions)?)
	}

	fn remove_batch(&self, iter: &mut dyn Iterator<Item = Vec<u8>>) -> Result<()> {
		let writeoptions = rust_rocksdb::WriteOptions::default();

		let mut batch = WriteBatchWithTransaction::<false>::default();

		for key in iter {
			batch.delete_cf(&self.cf(), key);
		}

		Ok(self.db.rocks.write_opt(batch, &writeoptions)?)
	}

	fn iter<'a>(&'a self) -> Box<dyn Iterator<Item = (Vec<u8>, Vec<u8>)> + 'a> {
		let mut readoptions = rust_rocksdb::ReadOptions::default();
		readoptions.set_total_order_seek(true);

		Box::new(
			self.db
				.rocks
				.iterator_cf_opt(&self.cf(), readoptions, rust_rocksdb::IteratorMode::Start)
				.map(std::result::Result::unwrap)
				.map(|(k, v)| (Vec::from(k), Vec::from(v))),
		)
	}

	fn iter_from<'a>(&'a self, from: &[u8], backwards: bool) -> Box<dyn Iterator<Item = (Vec<u8>, Vec<u8>)> + 'a> {
		let mut readoptions = rust_rocksdb::ReadOptions::default();
		readoptions.set_total_order_seek(true);

		Box::new(
			self.db
				.rocks
				.iterator_cf_opt(
					&self.cf(),
					readoptions,
					rust_rocksdb::IteratorMode::From(
						from,
						if backwards {
							rust_rocksdb::Direction::Reverse
						} else {
							rust_rocksdb::Direction::Forward
						},
					),
				)
				.map(std::result::Result::unwrap)
				.map(|(k, v)| (Vec::from(k), Vec::from(v))),
		)
	}

	fn increment(&self, key: &[u8]) -> Result<Vec<u8>> {
		let mut readoptions = rust_rocksdb::ReadOptions::default();
		readoptions.set_total_order_seek(true);
		let writeoptions = rust_rocksdb::WriteOptions::default();

		let lock = self.write_lock.write().unwrap();

		let old = self.db.rocks.get_cf_opt(&self.cf(), key, &readoptions)?;
		let new = utils::increment(old.as_deref()).unwrap();
		self.db.rocks.put_cf_opt(&self.cf(), key, &new, &writeoptions)?;

		drop(lock);
		Ok(new)
	}

	fn increment_batch(&self, iter: &mut dyn Iterator<Item = Vec<u8>>) -> Result<()> {
		let mut readoptions = rust_rocksdb::ReadOptions::default();
		readoptions.set_total_order_seek(true);
		let writeoptions = rust_rocksdb::WriteOptions::default();

		let mut batch = WriteBatchWithTransaction::<false>::default();

		let lock = self.write_lock.write().unwrap();

		for key in iter {
			let old = self.db.rocks.get_cf_opt(&self.cf(), &key, &readoptions)?;
			let new = utils::increment(old.as_deref()).unwrap();
			batch.put_cf(&self.cf(), key, new);
		}

		self.db.rocks.write_opt(batch, &writeoptions)?;

		drop(lock);

		Ok(())
	}

	fn scan_prefix<'a>(&'a self, prefix: Vec<u8>) -> Box<dyn Iterator<Item = (Vec<u8>, Vec<u8>)> + 'a> {
		let mut readoptions = rust_rocksdb::ReadOptions::default();
		readoptions.set_total_order_seek(true);

		Box::new(
			self.db
				.rocks
				.iterator_cf_opt(
					&self.cf(),
					readoptions,
					rust_rocksdb::IteratorMode::From(&prefix, rust_rocksdb::Direction::Forward),
				)
				.map(std::result::Result::unwrap)
				.map(|(k, v)| (Vec::from(k), Vec::from(v)))
				.take_while(move |(k, _)| k.starts_with(&prefix)),
		)
	}

	fn watch_prefix<'a>(&'a self, prefix: &[u8]) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
		self.watchers.watch(prefix)
	}
}
