use std::{
	future::Future,
	pin::Pin,
	sync::{Arc, RwLock},
};

use rust_rocksdb::{
	LogLevel::{Debug, Error, Fatal, Info, Warn},
	WriteBatchWithTransaction,
};
use tracing::{debug, info};

use super::{super::Config, watchers::Watchers, KeyValueDatabaseEngine, KvTree};
use crate::{utils, Result};

pub(crate) struct Engine {
	rocks: rust_rocksdb::DBWithThreadMode<rust_rocksdb::MultiThreaded>,
	cache: rust_rocksdb::Cache,
	old_cfs: Vec<String>,
	config: Config,
}

struct RocksDbEngineTree<'a> {
	db: Arc<Engine>,
	name: &'a str,
	watchers: Watchers,
	write_lock: RwLock<()>,
}

fn db_options(rocksdb_cache: &rust_rocksdb::Cache, config: &Config) -> rust_rocksdb::Options {
	// block-based options: https://docs.rs/rocksdb/latest/rocksdb/struct.BlockBasedOptions.html#
	let mut block_based_options = rust_rocksdb::BlockBasedOptions::default();

	block_based_options.set_block_cache(rocksdb_cache);

	// "Difference of spinning disk"
	// https://zhangyuchi.gitbooks.io/rocksdbbook/content/RocksDB-Tuning-Guide.html
	block_based_options.set_block_size(64 * 1024);
	block_based_options.set_cache_index_and_filter_blocks(true);

	block_based_options.set_bloom_filter(10.0, false);
	block_based_options.set_pin_l0_filter_and_index_blocks_in_cache(true);
	block_based_options.set_optimize_filters_for_memory(true);

	// database options: https://docs.rs/rocksdb/latest/rocksdb/struct.Options.html#
	let mut db_opts = rust_rocksdb::Options::default();

	let rocksdb_log_level = match config.rocksdb_log_level.as_ref() {
		"debug" => Debug,
		"info" => Info,
		"warn" => Warn,
		"fatal" => Fatal,
		_ => Error,
	};

	let rocksdb_compression_algo = match config.rocksdb_compression_algo.as_ref() {
		"zstd" => rust_rocksdb::DBCompressionType::Zstd,
		"zlib" => rust_rocksdb::DBCompressionType::Zlib,
		"lz4" => rust_rocksdb::DBCompressionType::Lz4,
		"bz2" => rust_rocksdb::DBCompressionType::Bz2,
		_ => rust_rocksdb::DBCompressionType::Zstd,
	};

	let threads = if config.rocksdb_parallelism_threads == 0 {
		num_cpus::get_physical() // max cores if user specified 0
	} else {
		config.rocksdb_parallelism_threads
	};

	db_opts.set_log_level(rocksdb_log_level);
	db_opts.set_max_log_file_size(config.rocksdb_max_log_file_size);
	db_opts.set_log_file_time_to_roll(config.rocksdb_log_time_to_roll);
	db_opts.set_keep_log_file_num(config.rocksdb_max_log_files);

	if config.rocksdb_optimize_for_spinning_disks {
		db_opts.set_skip_stats_update_on_db_open(true); // speeds up opening DB on hard drives
		db_opts.set_compaction_readahead_size(4 * 1024 * 1024); // "If you’re running RocksDB on spinning disks, you should set this to at least
														// 2MB. That way RocksDB’s compaction is doing sequential instead of random
														// reads."
		db_opts.set_target_file_size_base(256 * 1024 * 1024);
	} else {
		db_opts.set_max_bytes_for_level_base(512 * 1024 * 1024);
		db_opts.set_use_direct_reads(true);
		db_opts.set_use_direct_io_for_flush_and_compaction(true);
	}

	if config.rocksdb_bottommost_compression {
		db_opts.set_bottommost_compression_type(rocksdb_compression_algo);
		db_opts.set_bottommost_zstd_max_train_bytes(0, true);

		// -14 w_bits is only read by zlib.
		db_opts.set_bottommost_compression_options(-14, config.rocksdb_bottommost_compression_level, 0, 0, true);
	}

	// -14 w_bits is only read by zlib.
	db_opts.set_compression_options(-14, config.rocksdb_compression_level, 0, 0);

	db_opts.set_block_based_table_factory(&block_based_options);
	db_opts.create_if_missing(true);
	db_opts.increase_parallelism(
		threads.try_into().expect("Failed to convert \"rocksdb_parallelism_threads\" usize into i32"),
	);
	db_opts.set_compression_type(rocksdb_compression_algo);

	// https://github.com/facebook/rocksdb/wiki/Setup-Options-and-Basic-Tuning
	db_opts.set_level_compaction_dynamic_level_bytes(true);
	db_opts.set_max_background_jobs(6);
	db_opts.set_bytes_per_sync(1_048_576);

	// https://github.com/facebook/rocksdb/wiki/WAL-Recovery-Modes#ktoleratecorruptedtailrecords
	//
	// Unclean shutdowns of a Matrix homeserver are likely to be fine when
	// recovered in this manner as it's likely any lost information will be
	// restored via federation.
	db_opts.set_wal_recovery_mode(rust_rocksdb::DBRecoveryMode::TolerateCorruptedTailRecords);

	// TODO: remove me? https://gitlab.com/famedly/conduit/-/merge_requests/602/diffs#a3a261d6a9014330581b5bdecd586dab5ae00245_62_54
	let prefix_extractor = rust_rocksdb::SliceTransform::create_fixed_prefix(1);
	db_opts.set_prefix_extractor(prefix_extractor);

	db_opts
}

impl KeyValueDatabaseEngine for Arc<Engine> {
	fn open(config: &Config) -> Result<Self> {
		let cache_capacity_bytes = (config.db_cache_capacity_mb * 1024.0 * 1024.0) as usize;
		let rocksdb_cache = rust_rocksdb::Cache::new_lru_cache(cache_capacity_bytes);

		let db_opts = db_options(&rocksdb_cache, config);

		debug!("Listing column families in database");
		let cfs =
			rust_rocksdb::DBWithThreadMode::<rust_rocksdb::MultiThreaded>::list_cf(&db_opts, &config.database_path)
				.unwrap_or_default();

		debug!("Opening column family descriptors in database");
		info!("RocksDB database compaction will take place now, a delay in startup is expected");
		let db = rust_rocksdb::DBWithThreadMode::<rust_rocksdb::MultiThreaded>::open_cf_descriptors(
			&db_opts,
			&config.database_path,
			cfs.iter().map(|name| rust_rocksdb::ColumnFamilyDescriptor::new(name, db_options(&rocksdb_cache, config))),
		)?;

		Ok(Arc::new(Engine {
			rocks: db,
			cache: rocksdb_cache,
			old_cfs: cfs,
			config: config.clone(),
		}))
	}

	fn open_tree(&self, name: &'static str) -> Result<Arc<dyn KvTree>> {
		if !self.old_cfs.contains(&name.to_owned()) {
			// Create if it didn't exist
			debug!("Creating new column family in database: {}", name);
			let _ = self.rocks.create_cf(name, &db_options(&self.cache, &self.config));
		}

		Ok(Arc::new(RocksDbEngineTree {
			name,
			db: Arc::clone(self),
			watchers: Watchers::default(),
			write_lock: RwLock::new(()),
		}))
	}

	fn flush(&self) -> Result<()> {
		debug!("Running flush_wal (no sync)");
		rust_rocksdb::DBCommon::flush_wal(&self.rocks, false)?;

		Ok(())
	}

	fn memory_usage(&self) -> Result<String> {
		let stats = rust_rocksdb::perf::get_memory_usage_stats(Some(&[&self.rocks]), Some(&[&self.cache]))?;
		Ok(format!(
			"Approximate memory usage of all the mem-tables: {:.3} MB\nApproximate memory usage of un-flushed \
			 mem-tables: {:.3} MB\nApproximate memory usage of all the table readers: {:.3} MB\nApproximate memory \
			 usage by cache: {:.3} MB\nApproximate memory usage by cache pinned: {:.3} MB\n",
			stats.mem_table_total as f64 / 1024.0 / 1024.0,
			stats.mem_table_unflushed as f64 / 1024.0 / 1024.0,
			stats.mem_table_readers_total as f64 / 1024.0 / 1024.0,
			stats.cache_total as f64 / 1024.0 / 1024.0,
			self.cache.get_pinned_usage() as f64 / 1024.0 / 1024.0,
		))
	}

	fn cleanup(&self) -> Result<()> {
		debug!("Running flush_opt");
		let flushoptions = rust_rocksdb::FlushOptions::default();

		rust_rocksdb::DBCommon::flush_opt(&self.rocks, &flushoptions)?;

		Ok(())
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
