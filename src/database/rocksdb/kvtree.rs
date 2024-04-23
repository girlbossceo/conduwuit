use std::{future::Future, pin::Pin, sync::Arc};

use rust_rocksdb::WriteBatchWithTransaction;

use super::{watchers::Watchers, Engine, KeyValueDatabaseEngine, KvTree};
use crate::{utils, Result};

pub(crate) struct RocksDbEngineTree<'a> {
	pub(crate) db: Arc<Engine>,
	pub(crate) name: &'a str,
	pub(crate) watchers: Watchers,
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

	fn multi_get(&self, keys: &[&[u8]]) -> Result<Vec<Option<Vec<u8>>>> {
		let mut readoptions = rust_rocksdb::ReadOptions::default();
		readoptions.set_total_order_seek(true);

		// Optimization can be `true` if key vector is pre-sorted **by the column
		// comparator**.
		const SORTED: bool = false;

		let mut ret: Vec<Option<Vec<u8>>> = Vec::with_capacity(keys.len());
		for res in self
			.db
			.rocks
			.batched_multi_get_cf_opt(&self.cf(), keys, SORTED, &readoptions)
		{
			match res? {
				Some(res) => ret.push(Some((*res).to_vec())),
				None => ret.push(None),
			}
		}

		Ok(ret)
	}

	fn insert(&self, key: &[u8], value: &[u8]) -> Result<()> {
		let writeoptions = rust_rocksdb::WriteOptions::default();

		self.db
			.rocks
			.put_cf_opt(&self.cf(), key, value, &writeoptions)?;

		if !self.db.corked() {
			self.db.flush()?;
		}

		self.watchers.wake(key);

		Ok(())
	}

	fn insert_batch(&self, iter: &mut dyn Iterator<Item = (Vec<u8>, Vec<u8>)>) -> Result<()> {
		let writeoptions = rust_rocksdb::WriteOptions::default();

		let mut batch = WriteBatchWithTransaction::<false>::default();

		for (key, value) in iter {
			batch.put_cf(&self.cf(), key, value);
		}

		let result = self.db.rocks.write_opt(batch, &writeoptions);

		if !self.db.corked() {
			self.db.flush()?;
		}

		Ok(result?)
	}

	fn remove(&self, key: &[u8]) -> Result<()> {
		let writeoptions = rust_rocksdb::WriteOptions::default();

		let result = self.db.rocks.delete_cf_opt(&self.cf(), key, &writeoptions);

		if !self.db.corked() {
			self.db.flush()?;
		}

		Ok(result?)
	}

	fn remove_batch(&self, iter: &mut dyn Iterator<Item = Vec<u8>>) -> Result<()> {
		let writeoptions = rust_rocksdb::WriteOptions::default();

		let mut batch = WriteBatchWithTransaction::<false>::default();

		for key in iter {
			batch.delete_cf(&self.cf(), key);
		}

		let result = self.db.rocks.write_opt(batch, &writeoptions);

		if !self.db.corked() {
			self.db.flush()?;
		}

		Ok(result?)
	}

	fn iter<'a>(&'a self) -> Box<dyn Iterator<Item = (Vec<u8>, Vec<u8>)> + 'a> {
		let mut readoptions = rust_rocksdb::ReadOptions::default();
		readoptions.set_total_order_seek(true);

		Box::new(
			self.db
				.rocks
				.iterator_cf_opt(&self.cf(), readoptions, rust_rocksdb::IteratorMode::Start)
				.map(Result::unwrap)
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
				.map(Result::unwrap)
				.map(|(k, v)| (Vec::from(k), Vec::from(v))),
		)
	}

	fn increment(&self, key: &[u8]) -> Result<Vec<u8>> {
		let mut readoptions = rust_rocksdb::ReadOptions::default();
		readoptions.set_total_order_seek(true);
		let writeoptions = rust_rocksdb::WriteOptions::default();

		let old = self.db.rocks.get_cf_opt(&self.cf(), key, &readoptions)?;
		let new = utils::increment(old.as_deref());
		self.db
			.rocks
			.put_cf_opt(&self.cf(), key, &new, &writeoptions)?;

		if !self.db.corked() {
			self.db.flush()?;
		}

		Ok(new)
	}

	fn increment_batch(&self, iter: &mut dyn Iterator<Item = Vec<u8>>) -> Result<()> {
		let mut readoptions = rust_rocksdb::ReadOptions::default();
		readoptions.set_total_order_seek(true);
		let writeoptions = rust_rocksdb::WriteOptions::default();

		let mut batch = WriteBatchWithTransaction::<false>::default();

		for key in iter {
			let old = self.db.rocks.get_cf_opt(&self.cf(), &key, &readoptions)?;
			let new = utils::increment(old.as_deref());
			batch.put_cf(&self.cf(), key, new);
		}

		self.db.rocks.write_opt(batch, &writeoptions)?;

		if !self.db.corked() {
			self.db.flush()?;
		}

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
				.map(Result::unwrap)
				.map(|(k, v)| (Vec::from(k), Vec::from(v)))
				.take_while(move |(k, _)| k.starts_with(&prefix)),
		)
	}

	fn watch_prefix<'a>(&'a self, prefix: &[u8]) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
		self.watchers.watch(prefix)
	}
}
