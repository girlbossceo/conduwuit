mod count;
mod get;
mod insert;
mod keys;
mod keys_from;
mod keys_prefix;
mod remove;
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
	pin::Pin,
	sync::Arc,
};

use conduit::Result;
use rocksdb::{AsColumnFamilyRef, ColumnFamily, ReadOptions, WriteOptions};

use crate::{watchers::Watchers, Engine};

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

	// SAFETY: Column family handles out of RocksDB are basic pointers and can
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
