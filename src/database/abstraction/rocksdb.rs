use super::{super::Config, watchers::Watchers, KeyValueDatabaseEngine, KvTree};
use crate::{utils, Result};
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, RwLock},
};

use rocksdb::LogLevel::{Debug, Error, Fatal, Info, Warn};
use tracing::{debug, info};

pub(crate) struct Engine {
    rocks: rocksdb::DBWithThreadMode<rocksdb::MultiThreaded>,
    cache: rocksdb::Cache,
    old_cfs: Vec<String>,
    config: Config,
}

struct RocksDbEngineTree<'a> {
    db: Arc<Engine>,
    name: &'a str,
    watchers: Watchers,
    write_lock: RwLock<()>,
}

fn db_options(rocksdb_cache: &rocksdb::Cache, config: &Config) -> rocksdb::Options {
    // block-based options: https://docs.rs/rocksdb/latest/rocksdb/struct.BlockBasedOptions.html#
    let mut block_based_options = rocksdb::BlockBasedOptions::default();

    block_based_options.set_block_cache(rocksdb_cache);

    // "Difference of spinning disk"
    // https://zhangyuchi.gitbooks.io/rocksdbbook/content/RocksDB-Tuning-Guide.html
    block_based_options.set_block_size(64 * 1024);
    block_based_options.set_cache_index_and_filter_blocks(true);

    // database options: https://docs.rs/rocksdb/latest/rocksdb/struct.Options.html#
    let mut db_opts = rocksdb::Options::default();

    let rocksdb_log_level = match config.rocksdb_log_level.as_ref() {
        "debug" => Debug,
        "info" => Info,
        "warn" => Warn,
        "error" => Error,
        "fatal" => Fatal,
        _ => Warn,
    };

    db_opts.set_log_level(rocksdb_log_level);
    db_opts.set_max_log_file_size(config.rocksdb_max_log_file_size);
    db_opts.set_log_file_time_to_roll(config.rocksdb_log_time_to_roll);

    if config.rocksdb_optimize_for_spinning_disks {
        // useful for hard drives but on literally any half-decent SSD this is not useful
        // and the benefits of improved compaction based on up to date stats are good.
        // current conduwut users have NVMe/SSDs.
        db_opts.set_skip_stats_update_on_db_open(true);

        db_opts.set_compaction_readahead_size(2 * 1024 * 1024); // default compaction_readahead_size is 0 which is good for SSDs
        db_opts.set_target_file_size_base(256 * 1024 * 1024); // default target_file_size is 64MB which is good for SSDs
        db_opts.set_optimize_filters_for_hits(true); // doesn't really seem useful for fast storage
    } else {
        db_opts.set_skip_stats_update_on_db_open(false);
        db_opts.set_max_bytes_for_level_base(512 * 1024 * 1024);
        db_opts.set_use_direct_reads(true);
        db_opts.set_use_direct_io_for_flush_and_compaction(true);
    }

    db_opts.set_block_based_table_factory(&block_based_options);
    db_opts.set_level_compaction_dynamic_level_bytes(true);
    db_opts.create_if_missing(true);
    db_opts.increase_parallelism(num_cpus::get() as i32);
    //db_opts.set_max_open_files(config.rocksdb_max_open_files);
    db_opts.set_compression_type(rocksdb::DBCompressionType::Zstd);
    db_opts.set_compaction_style(rocksdb::DBCompactionStyle::Level);
    db_opts.optimize_level_style_compaction(10 * 1024 * 1024);

    // https://github.com/facebook/rocksdb/wiki/Setup-Options-and-Basic-Tuning
    db_opts.set_max_background_jobs(6);
    db_opts.set_bytes_per_sync(1048576);

    // https://github.com/facebook/rocksdb/wiki/WAL-Recovery-Modes#ktoleratecorruptedtailrecords
    //
    // Unclean shutdowns of a Matrix homeserver are likely to be fine when
    // recovered in this manner as it's likely any lost information will be
    // restored via federation.
    db_opts.set_wal_recovery_mode(rocksdb::DBRecoveryMode::TolerateCorruptedTailRecords);

    let prefix_extractor = rocksdb::SliceTransform::create_fixed_prefix(1);
    db_opts.set_prefix_extractor(prefix_extractor);

    db_opts
}

impl KeyValueDatabaseEngine for Arc<Engine> {
    fn open(config: &Config) -> Result<Self> {
        let cache_capacity_bytes = (config.db_cache_capacity_mb * 1024.0 * 1024.0) as usize;
        let rocksdb_cache = rocksdb::Cache::new_lru_cache(cache_capacity_bytes);

        let db_opts = db_options(&rocksdb_cache, config);

        debug!("Listing column families in database");
        let cfs = rocksdb::DBWithThreadMode::<rocksdb::MultiThreaded>::list_cf(
            &db_opts,
            &config.database_path,
        )
        .unwrap_or_default();

        debug!("Opening column family descriptors in database");
        info!("RocksDB database compaction will take place now, a delay in startup is expected");
        let db = rocksdb::DBWithThreadMode::<rocksdb::MultiThreaded>::open_cf_descriptors(
            &db_opts,
            &config.database_path,
            cfs.iter().map(|name| {
                rocksdb::ColumnFamilyDescriptor::new(name, db_options(&rocksdb_cache, config))
            }),
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
            let _ = self
                .rocks
                .create_cf(name, &db_options(&self.cache, &self.config));
        }

        Ok(Arc::new(RocksDbEngineTree {
            name,
            db: Arc::clone(self),
            watchers: Watchers::default(),
            write_lock: RwLock::new(()),
        }))
    }

    fn flush(&self) -> Result<()> {
        // TODO?
        Ok(())
    }

    fn memory_usage(&self) -> Result<String> {
        let stats =
            rocksdb::perf::get_memory_usage_stats(Some(&[&self.rocks]), Some(&[&self.cache]))?;
        Ok(format!(
            "Approximate memory usage of all the mem-tables: {:.3} MB\n\
             Approximate memory usage of un-flushed mem-tables: {:.3} MB\n\
             Approximate memory usage of all the table readers: {:.3} MB\n\
             Approximate memory usage by cache: {:.3} MB\n\
             Approximate memory usage by cache pinned: {:.3} MB\n\
             ",
            stats.mem_table_total as f64 / 1024.0 / 1024.0,
            stats.mem_table_unflushed as f64 / 1024.0 / 1024.0,
            stats.mem_table_readers_total as f64 / 1024.0 / 1024.0,
            stats.cache_total as f64 / 1024.0 / 1024.0,
            self.cache.get_pinned_usage() as f64 / 1024.0 / 1024.0,
        ))
    }

    // TODO: figure out if this is needed for rocksdb
    #[allow(dead_code)]
    fn clear_caches(&self) {}
}

impl RocksDbEngineTree<'_> {
    fn cf(&self) -> Arc<rocksdb::BoundColumnFamily<'_>> {
        self.db.rocks.cf_handle(self.name).unwrap()
    }
}

impl KvTree for RocksDbEngineTree<'_> {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        Ok(self.db.rocks.get_cf(&self.cf(), key)?)
    }

    fn insert(&self, key: &[u8], value: &[u8]) -> Result<()> {
        let lock = self.write_lock.read().unwrap();
        self.db.rocks.put_cf(&self.cf(), key, value)?;
        drop(lock);

        self.watchers.wake(key);

        Ok(())
    }

    fn insert_batch<'a>(&self, iter: &mut dyn Iterator<Item = (Vec<u8>, Vec<u8>)>) -> Result<()> {
        for (key, value) in iter {
            self.db.rocks.put_cf(&self.cf(), key, value)?;
        }

        Ok(())
    }

    fn remove(&self, key: &[u8]) -> Result<()> {
        Ok(self.db.rocks.delete_cf(&self.cf(), key)?)
    }

    fn iter<'a>(&'a self) -> Box<dyn Iterator<Item = (Vec<u8>, Vec<u8>)> + 'a> {
        Box::new(
            self.db
                .rocks
                .iterator_cf(&self.cf(), rocksdb::IteratorMode::Start)
                .map(|r| r.unwrap())
                .map(|(k, v)| (Vec::from(k), Vec::from(v))),
        )
    }

    fn iter_from<'a>(
        &'a self,
        from: &[u8],
        backwards: bool,
    ) -> Box<dyn Iterator<Item = (Vec<u8>, Vec<u8>)> + 'a> {
        Box::new(
            self.db
                .rocks
                .iterator_cf(
                    &self.cf(),
                    rocksdb::IteratorMode::From(
                        from,
                        if backwards {
                            rocksdb::Direction::Reverse
                        } else {
                            rocksdb::Direction::Forward
                        },
                    ),
                )
                .map(|r| r.unwrap())
                .map(|(k, v)| (Vec::from(k), Vec::from(v))),
        )
    }

    fn increment(&self, key: &[u8]) -> Result<Vec<u8>> {
        let lock = self.write_lock.write().unwrap();

        let old = self.db.rocks.get_cf(&self.cf(), key)?;
        let new = utils::increment(old.as_deref()).unwrap();
        self.db.rocks.put_cf(&self.cf(), key, &new)?;

        drop(lock);
        Ok(new)
    }

    fn increment_batch<'a>(&self, iter: &mut dyn Iterator<Item = Vec<u8>>) -> Result<()> {
        let lock = self.write_lock.write().unwrap();

        for key in iter {
            let old = self.db.rocks.get_cf(&self.cf(), &key)?;
            let new = utils::increment(old.as_deref()).unwrap();
            self.db.rocks.put_cf(&self.cf(), key, new)?;
        }

        drop(lock);

        Ok(())
    }

    fn scan_prefix<'a>(
        &'a self,
        prefix: Vec<u8>,
    ) -> Box<dyn Iterator<Item = (Vec<u8>, Vec<u8>)> + 'a> {
        Box::new(
            self.db
                .rocks
                .iterator_cf(
                    &self.cf(),
                    rocksdb::IteratorMode::From(&prefix, rocksdb::Direction::Forward),
                )
                .map(|r| r.unwrap())
                .map(|(k, v)| (Vec::from(k), Vec::from(v)))
                .take_while(move |(k, _)| k.starts_with(&prefix)),
        )
    }

    fn watch_prefix<'a>(&'a self, prefix: &[u8]) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        self.watchers.watch(prefix)
    }
}
