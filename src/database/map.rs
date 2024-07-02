use std::{future::Future, pin::Pin, sync::Arc};

use conduit::{utils, Result};
use rocksdb::{
	AsColumnFamilyRef, ColumnFamily, Direction, IteratorMode, ReadOptions, WriteBatchWithTransaction, WriteOptions,
};

use crate::{or_else, result, watchers::Watchers, Engine, Iter};

pub struct Map {
	name: String,
	db: Arc<Engine>,
	cf: Arc<ColumnFamily>,
	watchers: Watchers,
	write_options: WriteOptions,
	read_options: ReadOptions,
}

pub(crate) type KeyVal = (Key, Val);
pub(crate) type Val = Vec<u8>;
pub(crate) type Key = Vec<u8>;

impl Map {
	pub(crate) fn open(db: &Arc<Engine>, name: &str) -> Result<Arc<Self>> {
		Ok(Arc::new(Self {
			name: name.to_owned(),
			db: db.clone(),
			cf: open(db, name)?,
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
		let mode = IteratorMode::Start;
		let read_options = read_options_default();
		Box::new(Iter::new(&self.db, &self.cf, read_options, &mode))
	}

	pub fn iter_from<'a>(&'a self, from: &[u8], backwards: bool) -> Box<dyn Iterator<Item = KeyVal> + 'a> {
		let direction = if backwards {
			Direction::Reverse
		} else {
			Direction::Forward
		};
		let mode = IteratorMode::From(from, direction);
		let read_options = read_options_default();
		Box::new(Iter::new(&self.db, &self.cf, read_options, &mode))
	}

	pub fn scan_prefix<'a>(&'a self, prefix: Vec<u8>) -> Box<dyn Iterator<Item = KeyVal> + 'a> {
		let mode = IteratorMode::From(&prefix, Direction::Forward);
		let read_options = read_options_default();
		Box::new(Iter::new(&self.db, &self.cf, read_options, &mode).take_while(move |(k, _)| k.starts_with(&prefix)))
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

	pub fn watch_prefix<'a>(&'a self, prefix: &[u8]) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
		self.watchers.watch(prefix)
	}

	#[inline]
	pub fn name(&self) -> &str { &self.name }

	fn cf(&self) -> impl AsColumnFamilyRef + '_ { &*self.cf }
}

impl<'a> IntoIterator for &'a Map {
	type IntoIter = Box<dyn Iterator<Item = Self::Item> + 'a>;
	type Item = KeyVal;

	fn into_iter(self) -> Self::IntoIter { self.iter() }
}

fn open(db: &Arc<Engine>, name: &str) -> Result<Arc<ColumnFamily>> {
	let bounded_arc = db.open_cf(name)?;
	let bounded_ptr = Arc::into_raw(bounded_arc);
	let cf_ptr = bounded_ptr.cast::<ColumnFamily>();

	// SAFETY: After thorough contemplation this appears to be the best solution,
	// even by a significant margin.
	//
	// BACKGROUND: Column family handles out of RocksDB are basic pointers and can
	// be invalidated: 1. when the database closes. 2. when the column is dropped or
	// closed. rust_rocksdb wraps this for us by storing handles in their own
	// `RwLock<BTreeMap>` map and returning an Arc<BoundColumnFamily<'_>>` to
	// provide expected safety. Similarly in "single-threaded mode" we would
	// receive `&'_ ColumnFamily`.
	//
	// PROBLEM: We need to hold these handles in a field, otherwise we have to take
	// a lock and get them by name from this map for every query, which is what
	// conduit was doing, but we're not going to make a query for every query so we
	// need to be holding it right. The lifetime parameter on these references makes
	// that complicated. If this can be done without polluting the userspace
	// with lifetimes on every instance of `Map` then this `unsafe` might not be
	// necessary.
	//
	// SOLUTION: After investigating the underlying types it appears valid to
	// Arc-swap `BoundColumnFamily<'_>` for `ColumnFamily`. They have the
	// same inner data, the same Drop behavior, Deref, etc. We're just losing the
	// lifetime parameter. We should not hold this handle, even in its Arc, after
	// closing the database (dropping `Engine`). Since `Arc<Engine>` is a sibling
	// member along with this handle in `Map`, that is prevented.
	Ok(unsafe {
		Arc::decrement_strong_count(cf_ptr);
		Arc::from_raw(cf_ptr)
	})
}

#[inline]
fn read_options_default() -> ReadOptions {
	let mut read_options = ReadOptions::default();
	read_options.set_total_order_seek(true);
	read_options
}

#[inline]
fn write_options_default() -> WriteOptions { WriteOptions::default() }
