mod count;
mod keys;
mod keys_from;
mod keys_prefix;
mod rev_keys;
mod rev_keys_from;
mod rev_keys_prefix;
mod rev_stream;
mod rev_stream_from;
mod rev_stream_prefix;
mod stream;
mod stream_from;
mod stream_prefix;

use std::{
	convert::AsRef,
	ffi::CStr,
	fmt,
	fmt::{Debug, Display},
	future::Future,
	io::Write,
	pin::Pin,
	sync::Arc,
};

use conduit::{err, Result};
use futures::future;
use rocksdb::{AsColumnFamilyRef, ColumnFamily, ReadOptions, WriteBatchWithTransaction, WriteOptions};
use serde::Serialize;

use crate::{
	keyval::{OwnedKey, OwnedVal},
	ser,
	util::{map_err, or_else},
	watchers::Watchers,
	Engine, Handle,
};

pub struct Map {
	name: String,
	db: Arc<Engine>,
	cf: Arc<ColumnFamily>,
	watchers: Watchers,
	write_options: WriteOptions,
	read_options: ReadOptions,
}

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

	#[tracing::instrument(skip(self), fields(%self), level = "trace")]
	pub fn del<K>(&self, key: &K)
	where
		K: Serialize + ?Sized + Debug,
	{
		let mut buf = Vec::<u8>::with_capacity(64);
		self.bdel(key, &mut buf);
	}

	#[tracing::instrument(skip(self, buf), fields(%self), level = "trace")]
	pub fn bdel<K, B>(&self, key: &K, buf: &mut B)
	where
		K: Serialize + ?Sized + Debug,
		B: Write + AsRef<[u8]>,
	{
		let key = ser::serialize(buf, key).expect("failed to serialize deletion key");
		self.remove(&key);
	}

	#[tracing::instrument(level = "trace")]
	pub fn remove<K>(&self, key: &K)
	where
		K: AsRef<[u8]> + ?Sized + Debug,
	{
		let write_options = &self.write_options;
		self.db
			.db
			.delete_cf_opt(&self.cf(), key, write_options)
			.or_else(or_else)
			.expect("database remove error");

		if !self.db.corked() {
			self.db.flush().expect("database flush error");
		}
	}

	#[tracing::instrument(skip(self, value), fields(%self), level = "trace")]
	pub fn insert<K, V>(&self, key: &K, value: &V)
	where
		K: AsRef<[u8]> + ?Sized + Debug,
		V: AsRef<[u8]> + ?Sized,
	{
		let write_options = &self.write_options;
		self.db
			.db
			.put_cf_opt(&self.cf(), key, value, write_options)
			.or_else(or_else)
			.expect("database insert error");

		if !self.db.corked() {
			self.db.flush().expect("database flush error");
		}

		self.watchers.wake(key.as_ref());
	}

	#[tracing::instrument(skip(self), fields(%self), level = "trace")]
	pub fn insert_batch<'a, I, K, V>(&'a self, iter: I)
	where
		I: Iterator<Item = &'a (K, V)> + Send + Debug,
		K: AsRef<[u8]> + Sized + Debug + 'a,
		V: AsRef<[u8]> + Sized + 'a,
	{
		let mut batch = WriteBatchWithTransaction::<false>::default();
		for (key, val) in iter {
			batch.put_cf(&self.cf(), key.as_ref(), val.as_ref());
		}

		let write_options = &self.write_options;
		self.db
			.db
			.write_opt(batch, write_options)
			.or_else(or_else)
			.expect("database insert batch error");

		if !self.db.corked() {
			self.db.flush().expect("database flush error");
		}
	}

	#[tracing::instrument(skip(self), fields(%self), level = "trace")]
	pub fn qry<K>(&self, key: &K) -> impl Future<Output = Result<Handle<'_>>> + Send
	where
		K: Serialize + ?Sized + Debug,
	{
		let mut buf = Vec::<u8>::with_capacity(64);
		self.bqry(key, &mut buf)
	}

	#[tracing::instrument(skip(self, buf), fields(%self), level = "trace")]
	pub fn bqry<K, B>(&self, key: &K, buf: &mut B) -> impl Future<Output = Result<Handle<'_>>> + Send
	where
		K: Serialize + ?Sized + Debug,
		B: Write + AsRef<[u8]>,
	{
		let key = ser::serialize(buf, key).expect("failed to serialize query key");
		let val = self.get(key);
		future::ready(val)
	}

	#[tracing::instrument(skip(self), fields(%self), level = "trace")]
	pub fn get<K>(&self, key: &K) -> Result<Handle<'_>>
	where
		K: AsRef<[u8]> + ?Sized + Debug,
	{
		self.db
			.db
			.get_pinned_cf_opt(&self.cf(), key, &self.read_options)
			.map_err(map_err)?
			.map(Handle::from)
			.ok_or(err!(Request(NotFound("Not found in database"))))
	}

	#[tracing::instrument(skip(self), fields(%self), level = "trace")]
	pub fn multi_get<'a, I, K>(&self, keys: I) -> Vec<Option<OwnedVal>>
	where
		I: Iterator<Item = &'a K> + ExactSizeIterator + Send + Debug,
		K: AsRef<[u8]> + Sized + Debug + 'a,
	{
		// Optimization can be `true` if key vector is pre-sorted **by the column
		// comparator**.
		const SORTED: bool = false;

		let mut ret: Vec<Option<OwnedKey>> = Vec::with_capacity(keys.len());
		let read_options = &self.read_options;
		for res in self
			.db
			.db
			.batched_multi_get_cf_opt(&self.cf(), keys, SORTED, read_options)
		{
			match res {
				Ok(Some(res)) => ret.push(Some((*res).to_vec())),
				Ok(None) => ret.push(None),
				Err(e) => or_else(e).expect("database multiget error"),
			}
		}

		ret
	}

	#[inline]
	pub fn watch_prefix<'a, K>(&'a self, prefix: &K) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>
	where
		K: AsRef<[u8]> + ?Sized + Debug,
	{
		self.watchers.watch(prefix.as_ref())
	}

	#[inline]
	pub fn property_integer(&self, name: &CStr) -> Result<u64> { self.db.property_integer(&self.cf(), name) }

	#[inline]
	pub fn property(&self, name: &str) -> Result<String> { self.db.property(&self.cf(), name) }

	#[inline]
	pub fn name(&self) -> &str { &self.name }

	fn cf(&self) -> impl AsColumnFamilyRef + '_ { &*self.cf }
}

impl Debug for Map {
	fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result { write!(out, "Map {{name: {0}}}", self.name) }
}

impl Display for Map {
	fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result { write!(out, "{0}", self.name) }
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
		Arc::increment_strong_count(cf_ptr);
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
