use std::{future::Future, pin::Pin, sync::Arc};

use conduit::{utils, Result};
use rocksdb::{BoundColumnFamily, Direction, IteratorMode, ReadOptions, WriteBatchWithTransaction, WriteOptions};

use super::{or_else, result, watchers::Watchers, Engine};

pub struct Map {
	db: Arc<Engine>,
	name: String,
	watchers: Watchers,
}

type Key = Vec<u8>;
type Val = Vec<u8>;
type KeyVal = (Key, Val);

impl Map {
	pub(crate) fn open(db: &Arc<Engine>, name: &str) -> Result<Arc<Self>> {
		db.open_cf(name)?;
		Ok(Arc::new(Self {
			db: db.clone(),
			name: name.to_owned(),
			watchers: Watchers::default(),
		}))
	}

	pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
		let mut readoptions = ReadOptions::default();
		readoptions.set_total_order_seek(true);

		result(self.db.db.get_cf_opt(&self.cf(), key, &readoptions))
	}

	pub fn multi_get(&self, keys: &[&[u8]]) -> Result<Vec<Option<Vec<u8>>>> {
		// Optimization can be `true` if key vector is pre-sorted **by the column
		// comparator**.
		const SORTED: bool = false;

		let mut readoptions = ReadOptions::default();
		readoptions.set_total_order_seek(true);

		let mut ret: Vec<Option<Vec<u8>>> = Vec::with_capacity(keys.len());
		for res in self
			.db
			.db
			.batched_multi_get_cf_opt(&self.cf(), keys, SORTED, &readoptions)
		{
			match res {
				Ok(Some(res)) => ret.push(Some((*res).to_vec())),
				Ok(None) => ret.push(None),
				Err(e) => return or_else(e),
			}
		}

		Ok(ret)
	}

	pub fn insert(&self, key: &[u8], value: &[u8]) -> Result<()> {
		let writeoptions = WriteOptions::default();

		self.db
			.db
			.put_cf_opt(&self.cf(), key, value, &writeoptions)
			.or_else(or_else)?;

		if !self.db.corked() {
			self.db.flush()?;
		}

		self.watchers.wake(key);

		Ok(())
	}

	pub fn insert_batch(&self, iter: &mut dyn Iterator<Item = KeyVal>) -> Result<()> {
		let writeoptions = WriteOptions::default();

		let mut batch = WriteBatchWithTransaction::<false>::default();

		for (key, value) in iter {
			batch.put_cf(&self.cf(), key, value);
		}

		let res = self.db.db.write_opt(batch, &writeoptions);

		if !self.db.corked() {
			self.db.flush()?;
		}

		result(res)
	}

	pub fn remove(&self, key: &[u8]) -> Result<()> {
		let writeoptions = WriteOptions::default();

		let res = self.db.db.delete_cf_opt(&self.cf(), key, &writeoptions);

		if !self.db.corked() {
			self.db.flush()?;
		}

		result(res)
	}

	pub fn remove_batch(&self, iter: &mut dyn Iterator<Item = Key>) -> Result<()> {
		let writeoptions = WriteOptions::default();

		let mut batch = WriteBatchWithTransaction::<false>::default();

		for key in iter {
			batch.delete_cf(&self.cf(), key);
		}

		let res = self.db.db.write_opt(batch, &writeoptions);

		if !self.db.corked() {
			self.db.flush()?;
		}

		result(res)
	}

	pub fn iter<'a>(&'a self) -> Box<dyn Iterator<Item = KeyVal> + 'a> {
		let mut readoptions = ReadOptions::default();
		readoptions.set_total_order_seek(true);

		let it = self
			.db
			.db
			.iterator_cf_opt(&self.cf(), readoptions, IteratorMode::Start)
			.map(Result::unwrap)
			.map(|(k, v)| (Vec::from(k), Vec::from(v)));

		Box::new(it)
	}

	pub fn iter_from<'a>(&'a self, from: &[u8], backwards: bool) -> Box<dyn Iterator<Item = KeyVal> + 'a> {
		let mut readoptions = ReadOptions::default();
		readoptions.set_total_order_seek(true);

		let it = self
			.db
			.db
			.iterator_cf_opt(
				&self.cf(),
				readoptions,
				IteratorMode::From(
					from,
					if backwards {
						Direction::Reverse
					} else {
						Direction::Forward
					},
				),
			)
			.map(Result::unwrap)
			.map(|(k, v)| (Vec::from(k), Vec::from(v)));

		Box::new(it)
	}

	pub fn increment(&self, key: &[u8]) -> Result<Vec<u8>> {
		let mut readoptions = ReadOptions::default();
		readoptions.set_total_order_seek(true);
		let writeoptions = WriteOptions::default();

		let old = self
			.db
			.db
			.get_cf_opt(&self.cf(), key, &readoptions)
			.or_else(or_else)?;
		let new = utils::increment(old.as_deref());
		self.db
			.db
			.put_cf_opt(&self.cf(), key, new, &writeoptions)
			.or_else(or_else)?;

		if !self.db.corked() {
			self.db.flush()?;
		}

		Ok(new.to_vec())
	}

	pub fn increment_batch(&self, iter: &mut dyn Iterator<Item = Val>) -> Result<()> {
		let mut readoptions = ReadOptions::default();
		readoptions.set_total_order_seek(true);
		let writeoptions = WriteOptions::default();

		let mut batch = WriteBatchWithTransaction::<false>::default();

		for key in iter {
			let old = self
				.db
				.db
				.get_cf_opt(&self.cf(), &key, &readoptions)
				.or_else(or_else)?;
			let new = utils::increment(old.as_deref());
			batch.put_cf(&self.cf(), key, new);
		}

		self.db
			.db
			.write_opt(batch, &writeoptions)
			.or_else(or_else)?;

		if !self.db.corked() {
			self.db.flush()?;
		}

		Ok(())
	}

	pub fn scan_prefix<'a>(&'a self, prefix: Vec<u8>) -> Box<dyn Iterator<Item = KeyVal> + 'a> {
		let mut readoptions = ReadOptions::default();
		readoptions.set_total_order_seek(true);

		let it = self
			.db
			.db
			.iterator_cf_opt(&self.cf(), readoptions, IteratorMode::From(&prefix, Direction::Forward))
			.map(Result::unwrap)
			.map(|(k, v)| (Vec::from(k), Vec::from(v)))
			.take_while(move |(k, _)| k.starts_with(&prefix));

		Box::new(it)
	}

	pub fn watch_prefix<'a>(&'a self, prefix: &[u8]) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
		self.watchers.watch(prefix)
	}

	fn cf(&self) -> Arc<BoundColumnFamily<'_>> { self.db.cf(&self.name) }
}

impl<'a> IntoIterator for &'a Map {
	type IntoIter = Box<dyn Iterator<Item = Self::Item> + 'a>;
	type Item = KeyVal;

	fn into_iter(self) -> Self::IntoIter { self.iter() }
}
