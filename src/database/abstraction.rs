use std::{
    collections::BTreeMap,
    future::Future,
    pin::Pin,
    sync::{Arc, RwLock},
};

use log::warn;
use rocksdb::{
    BoundColumnFamily, ColumnFamilyDescriptor, DBWithThreadMode, Direction, MultiThreaded, Options,
};

use super::Config;
use crate::{utils, Result};

pub struct SledEngine(sled::Db);
pub struct SledEngineTree(sled::Tree);
pub struct RocksDbEngine(rocksdb::DBWithThreadMode<MultiThreaded>);
pub struct RocksDbEngineTree<'a> {
    db: Arc<RocksDbEngine>,
    name: &'a str,
    watchers: RwLock<BTreeMap<Vec<u8>, Vec<tokio::sync::oneshot::Sender<()>>>>,
}

pub trait DatabaseEngine: Sized {
    fn open(config: &Config) -> Result<Arc<Self>>;
    fn open_tree(self: &Arc<Self>, name: &'static str) -> Result<Arc<dyn Tree>>;
}

pub trait Tree: Send + Sync {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>>;

    fn insert(&self, key: &[u8], value: &[u8]) -> Result<()>;

    fn remove(&self, key: &[u8]) -> Result<()>;

    fn iter<'a>(&'a self) -> Box<dyn Iterator<Item = (Box<[u8]>, Box<[u8]>)> + Send + Sync + 'a>;

    fn iter_from<'a>(
        &'a self,
        from: &[u8],
        backwards: bool,
    ) -> Box<dyn Iterator<Item = (Box<[u8]>, Box<[u8]>)> + 'a>;

    fn increment(&self, key: &[u8]) -> Result<Vec<u8>>;

    fn scan_prefix<'a>(
        &'a self,
        prefix: Vec<u8>,
    ) -> Box<dyn Iterator<Item = (Box<[u8]>, Box<[u8]>)> + Send + 'a>;

    fn watch_prefix<'a>(&'a self, prefix: &[u8]) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

    fn clear(&self) -> Result<()> {
        for (key, _) in self.iter() {
            self.remove(&key)?;
        }

        Ok(())
    }
}

impl DatabaseEngine for SledEngine {
    fn open(config: &Config) -> Result<Arc<Self>> {
        Ok(Arc::new(SledEngine(
            sled::Config::default()
                .path(&config.database_path)
                .cache_capacity(config.cache_capacity as u64)
                .use_compression(true)
                .open()?,
        )))
    }

    fn open_tree(self: &Arc<Self>, name: &'static str) -> Result<Arc<dyn Tree>> {
        Ok(Arc::new(SledEngineTree(self.0.open_tree(name)?)))
    }
}

impl Tree for SledEngineTree {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        Ok(self.0.get(key)?.map(|v| v.to_vec()))
    }

    fn insert(&self, key: &[u8], value: &[u8]) -> Result<()> {
        self.0.insert(key, value)?;
        Ok(())
    }

    fn remove(&self, key: &[u8]) -> Result<()> {
        self.0.remove(key)?;
        Ok(())
    }

    fn iter<'a>(&'a self) -> Box<dyn Iterator<Item = (Box<[u8]>, Box<[u8]>)> + Send + Sync + 'a> {
        Box::new(
            self.0
                .iter()
                .filter_map(|r| {
                    if let Err(e) = &r {
                        warn!("Error: {}", e);
                    }
                    r.ok()
                })
                .map(|(k, v)| (k.to_vec().into(), v.to_vec().into())),
        )
    }

    fn iter_from(
        &self,
        from: &[u8],
        backwards: bool,
    ) -> Box<dyn Iterator<Item = (Box<[u8]>, Box<[u8]>)>> {
        let iter = if backwards {
            self.0.range(..from)
        } else {
            self.0.range(from..)
        };

        let iter = iter
            .filter_map(|r| {
                if let Err(e) = &r {
                    warn!("Error: {}", e);
                }
                r.ok()
            })
            .map(|(k, v)| (k.to_vec().into(), v.to_vec().into()));

        if backwards {
            Box::new(iter.rev())
        } else {
            Box::new(iter)
        }
    }

    fn increment(&self, key: &[u8]) -> Result<Vec<u8>> {
        Ok(self
            .0
            .update_and_fetch(key, utils::increment)
            .map(|o| o.expect("increment always sets a value").to_vec())?)
    }

    fn scan_prefix<'a>(
        &'a self,
        prefix: Vec<u8>,
    ) -> Box<dyn Iterator<Item = (Box<[u8]>, Box<[u8]>)> + Send + 'a> {
        let iter = self
            .0
            .scan_prefix(prefix)
            .filter_map(|r| {
                if let Err(e) = &r {
                    warn!("Error: {}", e);
                }
                r.ok()
            })
            .map(|(k, v)| (k.to_vec().into(), v.to_vec().into()));

        Box::new(iter)
    }

    fn watch_prefix<'a>(&'a self, prefix: &[u8]) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        let prefix = prefix.to_vec();
        Box::pin(async move {
            self.0.watch_prefix(prefix).await;
        })
    }
}

impl DatabaseEngine for RocksDbEngine {
    fn open(config: &Config) -> Result<Arc<Self>> {
        let mut db_opts = Options::default();
        db_opts.create_if_missing(true);

        let cfs = DBWithThreadMode::<MultiThreaded>::list_cf(&db_opts, &config.database_path)
            .unwrap_or_default();

        let mut options = Options::default();
        options.set_merge_operator_associative("increment", utils::increment_rocksdb);

        let db = DBWithThreadMode::<MultiThreaded>::open_cf_descriptors(
            &db_opts,
            &config.database_path,
            cfs.iter()
                .map(|name| ColumnFamilyDescriptor::new(name, options.clone())),
        )?;

        Ok(Arc::new(RocksDbEngine(db)))
    }

    fn open_tree(self: &Arc<Self>, name: &'static str) -> Result<Arc<dyn Tree>> {
        let mut options = Options::default();
        options.set_merge_operator_associative("increment", utils::increment_rocksdb);

        // Create if it doesn't exist
        let _ = self.0.create_cf(name, &options);

        Ok(Arc::new(RocksDbEngineTree {
            name,
            db: Arc::clone(self),
            watchers: RwLock::new(BTreeMap::new()),
        }))
    }
}

impl RocksDbEngineTree<'_> {
    fn cf(&self) -> BoundColumnFamily<'_> {
        self.db.0.cf_handle(self.name).unwrap()
    }
}

impl Tree for RocksDbEngineTree<'_> {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        Ok(self.db.0.get_cf(self.cf(), key)?)
    }

    fn insert(&self, key: &[u8], value: &[u8]) -> Result<()> {
        let watchers = self.watchers.read().unwrap();
        let mut triggered = Vec::new();

        for length in 0..=key.len() {
            if watchers.contains_key(&key[..length]) {
                triggered.push(&key[..length]);
            }
        }

        drop(watchers);

        if !triggered.is_empty() {
            let mut watchers = self.watchers.write().unwrap();
            for prefix in triggered {
                if let Some(txs) = watchers.remove(prefix) {
                    for tx in txs {
                        let _ = tx.send(());
                    }
                }
            }
        }

        Ok(self.db.0.put_cf(self.cf(), key, value)?)
    }

    fn remove(&self, key: &[u8]) -> Result<()> {
        Ok(self.db.0.delete_cf(self.cf(), key)?)
    }

    fn iter<'a>(&'a self) -> Box<dyn Iterator<Item = (Box<[u8]>, Box<[u8]>)> + Send + Sync + 'a> {
        Box::new(
            self.db
                .0
                .iterator_cf(self.cf(), rocksdb::IteratorMode::Start),
        )
    }

    fn iter_from<'a>(
        &'a self,
        from: &[u8],
        backwards: bool,
    ) -> Box<dyn Iterator<Item = (Box<[u8]>, Box<[u8]>)> + 'a> {
        Box::new(self.db.0.iterator_cf(
            self.cf(),
            rocksdb::IteratorMode::From(
                from,
                if backwards {
                    Direction::Reverse
                } else {
                    Direction::Forward
                },
            ),
        ))
    }

    fn increment(&self, key: &[u8]) -> Result<Vec<u8>> {
        // TODO: atomic?
        let old = self.get(key)?;
        let new = utils::increment(old.as_deref()).unwrap();
        self.insert(key, &new)?;
        Ok(new)
    }

    fn scan_prefix<'a>(
        &'a self,
        prefix: Vec<u8>,
    ) -> Box<dyn Iterator<Item = (Box<[u8]>, Box<[u8]>)> + Send + 'a> {
        Box::new(
            self.db
                .0
                .iterator_cf(
                    self.cf(),
                    rocksdb::IteratorMode::From(&prefix, Direction::Forward),
                )
                .take_while(move |(k, _)| k.starts_with(&prefix)),
        )
    }

    fn watch_prefix<'a>(&'a self, prefix: &[u8]) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        let (tx, rx) = tokio::sync::oneshot::channel();

        self.watchers
            .write()
            .unwrap()
            .entry(prefix.to_vec())
            .or_default()
            .push(tx);

        Box::pin(async move {
            // Tx is never destroyed
            rx.await.unwrap();
        })
    }
}
