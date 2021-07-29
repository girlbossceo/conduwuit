use super::super::Config;
use crossbeam::channel::{bounded, Sender as ChannelSender};
use threadpool::ThreadPool;

use crate::{Error, Result};
use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex, RwLock},
};
use tokio::sync::oneshot::Sender;

use super::{DatabaseEngine, Tree};

type TupleOfBytes = (Vec<u8>, Vec<u8>);

pub struct Engine {
    env: heed::Env,
    iter_pool: Mutex<ThreadPool>,
}

pub struct EngineTree {
    engine: Arc<Engine>,
    tree: Arc<heed::UntypedDatabase>,
    watchers: RwLock<HashMap<Vec<u8>, Vec<Sender<()>>>>,
}

fn convert_error(error: heed::Error) -> Error {
    panic!(error.to_string());
    Error::HeedError {
        error: error.to_string(),
    }
}

impl DatabaseEngine for Engine {
    fn open(config: &Config) -> Result<Arc<Self>> {
        let mut env_builder = heed::EnvOpenOptions::new();
        env_builder.map_size(1024 * 1024 * 1024 * 1024); // 1 Terabyte
        env_builder.max_readers(126);
        env_builder.max_dbs(128);
        unsafe {
            env_builder.flag(heed::flags::Flags::MdbNoSync);
            env_builder.flag(heed::flags::Flags::MdbNoMetaSync);
        }

        Ok(Arc::new(Engine {
            env: env_builder
                .open(&config.database_path)
                .map_err(convert_error)?,
            iter_pool: Mutex::new(ThreadPool::new(10)),
        }))
    }

    fn open_tree(self: &Arc<Self>, name: &'static str) -> Result<Arc<dyn Tree>> {
        // Creates the db if it doesn't exist already
        Ok(Arc::new(EngineTree {
            engine: Arc::clone(self),
            tree: Arc::new(
                self.env
                    .create_database(Some(name))
                    .map_err(convert_error)?,
            ),
            watchers: RwLock::new(HashMap::new()),
        }))
    }

    fn flush(self: &Arc<Self>) -> Result<()> {
        self.env.force_sync().map_err(convert_error)?;
        Ok(())
    }
}

impl EngineTree {
    #[tracing::instrument(skip(self, tree, from, backwards))]
    fn iter_from_thread(
        &self,
        tree: Arc<heed::UntypedDatabase>,
        from: Vec<u8>,
        backwards: bool,
    ) -> Box<dyn Iterator<Item = TupleOfBytes> + Send + Sync> {
        let (s, r) = bounded::<TupleOfBytes>(5);
        let engine = Arc::clone(&self.engine);

        let lock = self.engine.iter_pool.lock().unwrap();
        if lock.active_count() < lock.max_count() {
            lock.execute(move || {
                iter_from_thread_work(tree, &engine.env.read_txn().unwrap(), from, backwards, &s);
            });
        } else {
            std::thread::spawn(move || {
                iter_from_thread_work(tree, &engine.env.read_txn().unwrap(), from, backwards, &s);
            });
        }

        Box::new(r.into_iter())
    }
}

#[tracing::instrument(skip(tree, txn, from, backwards))]
fn iter_from_thread_work(
    tree: Arc<heed::UntypedDatabase>,
    txn: &heed::RoTxn<'_>,
    from: Vec<u8>,
    backwards: bool,
    s: &ChannelSender<(Vec<u8>, Vec<u8>)>,
) {
    if backwards {
        for (k, v) in tree.rev_range(txn, ..=&*from).unwrap().map(|r| r.unwrap()) {
            if s.send((k.to_vec(), v.to_vec())).is_err() {
                return;
            }
        }
    } else {
        if from.is_empty() {
            for (k, v) in tree.iter(txn).unwrap().map(|r| r.unwrap()) {
                if s.send((k.to_vec(), v.to_vec())).is_err() {
                    return;
                }
            }
        } else {
            for (k, v) in tree.range(txn, &*from..).unwrap().map(|r| r.unwrap()) {
                if s.send((k.to_vec(), v.to_vec())).is_err() {
                    return;
                }
            }
        }
    }
}

impl Tree for EngineTree {
    #[tracing::instrument(skip(self, key))]
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        let txn = self.engine.env.read_txn().map_err(convert_error)?;
        Ok(self
            .tree
            .get(&txn, &key)
            .map_err(convert_error)?
            .map(|s| s.to_vec()))
    }

    #[tracing::instrument(skip(self, key, value))]
    fn insert(&self, key: &[u8], value: &[u8]) -> Result<()> {
        let mut txn = self.engine.env.write_txn().map_err(convert_error)?;
        self.tree
            .put(&mut txn, &key, &value)
            .map_err(convert_error)?;
        txn.commit().map_err(convert_error)?;

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
        };

        Ok(())
    }

    #[tracing::instrument(skip(self, key))]
    fn remove(&self, key: &[u8]) -> Result<()> {
        let mut txn = self.engine.env.write_txn().map_err(convert_error)?;
        self.tree.delete(&mut txn, &key).map_err(convert_error)?;
        txn.commit().map_err(convert_error)?;
        Ok(())
    }

    #[tracing::instrument(skip(self))]
    fn iter<'a>(&'a self) -> Box<dyn Iterator<Item = (Vec<u8>, Vec<u8>)> + Send + 'a> {
        self.iter_from(&[], false)
    }

    #[tracing::instrument(skip(self, from, backwards))]
    fn iter_from(
        &self,
        from: &[u8],
        backwards: bool,
    ) -> Box<dyn Iterator<Item = (Vec<u8>, Vec<u8>)> + Send> {
        self.iter_from_thread(Arc::clone(&self.tree), from.to_vec(), backwards)
    }

    #[tracing::instrument(skip(self, key))]
    fn increment(&self, key: &[u8]) -> Result<Vec<u8>> {
        let mut txn = self.engine.env.write_txn().map_err(convert_error)?;

        let old = self.tree.get(&txn, &key).map_err(convert_error)?;
        let new =
            crate::utils::increment(old.as_deref()).expect("utils::increment always returns Some");

        self.tree
            .put(&mut txn, &key, &&*new)
            .map_err(convert_error)?;

        txn.commit().map_err(convert_error)?;

        Ok(new)
    }

    #[tracing::instrument(skip(self, prefix))]
    fn scan_prefix<'a>(
        &'a self,
        prefix: Vec<u8>,
    ) -> Box<dyn Iterator<Item = (Vec<u8>, Vec<u8>)> + Send + 'a> {
        Box::new(
            self.iter_from(&prefix, false)
                .take_while(move |(key, _)| key.starts_with(&prefix)),
        )
    }

    #[tracing::instrument(skip(self, prefix))]
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
