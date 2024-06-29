use std::{future::Future, pin::Pin, sync::Arc};

use conduit::{utils, Result};
use rocksdb::{BoundColumnFamily, Direction, IteratorMode, ReadOptions, WriteBatchWithTransaction, WriteOptions};

use super::{or_else, result, watchers::Watchers, Engine};

pub struct Map {
	db: Arc<Engine>,
	name: String,
	watchers: Watchers,
	write_options: WriteOptions,
	read_options: ReadOptions,
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
			write_options: write_options_default(),
			read_options: read_options_default(),
		}))
	}

	pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
		let read_options = &self.read_options;
		let res = self.db.db.get_cf_opt(&self.cf(), key, read_options);

		result(res)
	}

	pub fn multi_get(&self, keys: &[&[u8]]) -> Result<Vec<Option<Vec<u8>>>> {
		// Optimization can be `true` if key vector is pre-sorted **by the column
		// comparator**.
		const SORTED: bool = false;

		let mut ret: Vec<Option<Vec<u8>>> = Vec::with_capacity(keys.len());
		let read_options = &self.read_options;
		for res in self
			.db
			.db
			.batched_multi_get_cf_opt(&self.cf(), keys, SORTED, read_options)
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
		let write_options = &self.write_options;
		self.db
			.db
			.put_cf_opt(&self.cf(), key, value, write_options)
			.or_else(or_else)?;

		if !self.db.corked() {
			self.db.flush()?;
		}

		self.watchers.wake(key);

		Ok(())
	}

	pub fn insert_batch(&self, iter: &mut dyn Iterator<Item = KeyVal>) -> Result<()> {
		let mut batch = WriteBatchWithTransaction::<false>::default();
		for (key, value) in iter {
			batch.put_cf(&self.cf(), key, value);
		}

		let write_options = &self.write_options;
		let res = self.db.db.write_opt(batch, write_options);

		if !self.db.corked() {
			self.db.flush()?;
		}

		result(res)
	}

	pub fn remove(&self, key: &[u8]) -> Result<()> {
		let write_options = &self.write_options;
		let res = self.db.db.delete_cf_opt(&self.cf(), key, write_options);

		if !self.db.corked() {
			self.db.flush()?;
		}

		result(res)
	}

	pub fn remove_batch(&self, iter: &mut dyn Iterator<Item = Key>) -> Result<()> {
		let mut batch = WriteBatchWithTransaction::<false>::default();
		for key in iter {
			batch.delete_cf(&self.cf(), key);
		}

		let write_options = &self.write_options;
		let res = self.db.db.write_opt(batch, write_options);

		if !self.db.corked() {
			self.db.flush()?;
		}

		result(res)
	}

	pub fn iter<'a>(&'a self) -> Box<dyn Iterator<Item = KeyVal> + 'a> {
		let read_options = read_options_default();
		let it = self
			.db
			.db
			.iterator_cf_opt(&self.cf(), read_options, IteratorMode::Start)
			.map(Result::unwrap)
			.map(|(k, v)| (Vec::from(k), Vec::from(v)));

		Box::new(it)
	}

	pub fn iter_from<'a>(&'a self, from: &[u8], backwards: bool) -> Box<dyn Iterator<Item = KeyVal> + 'a> {
		let read_options = read_options_default();
		let it = self
			.db
			.db
			.iterator_cf_opt(
				&self.cf(),
				read_options,
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
		let read_options = &self.read_options;
		let old = self
			.db
			.db
			.get_cf_opt(&self.cf(), key, read_options)
			.or_else(or_else)?;

		let new = utils::increment(old.as_deref());

		let write_options = &self.write_options;
		self.db
			.db
			.put_cf_opt(&self.cf(), key, new, write_options)
			.or_else(or_else)?;

		if !self.db.corked() {
			self.db.flush()?;
		}

		Ok(new.to_vec())
	}

	pub fn increment_batch(&self, iter: &mut dyn Iterator<Item = Val>) -> Result<()> {
		let mut batch = WriteBatchWithTransaction::<false>::default();

		let read_options = &self.read_options;
		for key in iter {
			let old = self
				.db
				.db
				.get_cf_opt(&self.cf(), &key, read_options)
				.or_else(or_else)?;
			let new = utils::increment(old.as_deref());
			batch.put_cf(&self.cf(), key, new);
		}

		let write_options = &self.write_options;
		self.db
			.db
			.write_opt(batch, write_options)
			.or_else(or_else)?;

		if !self.db.corked() {
			self.db.flush()?;
		}

		Ok(())
	}

	pub fn scan_prefix<'a>(&'a self, prefix: Vec<u8>) -> Box<dyn Iterator<Item = KeyVal> + 'a> {
		let read_options = read_options_default();
		let it = self
			.db
			.db
			.iterator_cf_opt(&self.cf(), read_options, IteratorMode::From(&prefix, Direction::Forward))
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

#[inline]
fn read_options_default() -> ReadOptions {
	let mut read_options = ReadOptions::default();
	read_options.set_total_order_seek(true);
	read_options
}

#[inline]
fn write_options_default() -> WriteOptions { WriteOptions::default() }
