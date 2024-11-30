use std::{convert::AsRef, fmt::Debug, io::Write, sync::Arc};

use arrayvec::ArrayVec;
use conduit::{err, implement, utils::IterStream, Err, Result};
use futures::{future, Future, FutureExt, Stream, StreamExt};
use rocksdb::DBPinnableSlice;
use serde::Serialize;

use crate::{
	keyval::KeyBuf,
	ser,
	util::{is_incomplete, map_err, or_else},
	Handle,
};

type RocksdbResult<'a> = Result<Option<DBPinnableSlice<'a>>, rocksdb::Error>;

/// Fetch a value from the database into cache, returning a reference-handle
/// asynchronously. The key is serialized into an allocated buffer to perform
/// the query.
#[implement(super::Map)]
#[inline]
pub fn qry<K>(self: &Arc<Self>, key: &K) -> impl Future<Output = Result<Handle<'_>>> + Send
where
	K: Serialize + ?Sized + Debug,
{
	let mut buf = KeyBuf::new();
	self.bqry(key, &mut buf)
}

/// Fetch a value from the database into cache, returning a reference-handle
/// asynchronously. The key is serialized into a fixed-sized buffer to perform
/// the query. The maximum size is supplied as const generic parameter.
#[implement(super::Map)]
#[inline]
pub fn aqry<const MAX: usize, K>(self: &Arc<Self>, key: &K) -> impl Future<Output = Result<Handle<'_>>> + Send
where
	K: Serialize + ?Sized + Debug,
{
	let mut buf = ArrayVec::<u8, MAX>::new();
	self.bqry(key, &mut buf)
}

/// Fetch a value from the database into cache, returning a reference-handle
/// asynchronously. The key is serialized into a user-supplied Writer.
#[implement(super::Map)]
#[tracing::instrument(skip(self, buf), level = "trace")]
pub fn bqry<K, B>(self: &Arc<Self>, key: &K, buf: &mut B) -> impl Future<Output = Result<Handle<'_>>> + Send
where
	K: Serialize + ?Sized + Debug,
	B: Write + AsRef<[u8]>,
{
	let key = ser::serialize(buf, key).expect("failed to serialize query key");
	self.get(key)
}

#[implement(super::Map)]
#[tracing::instrument(skip(self, keys), fields(%self), level = "trace")]
pub fn get_batch<'a, I, K>(self: &'a Arc<Self>, keys: I) -> impl Stream<Item = Result<Handle<'_>>> + Send + 'a
where
	I: Iterator<Item = &'a K> + ExactSizeIterator + Debug + Send + 'a,
	K: AsRef<[u8]> + Debug + Send + ?Sized + Sync + 'a,
{
	keys.stream()
		.map(move |key| self.get(key))
		.buffered(self.db.server.config.db_pool_workers.saturating_mul(2))
}

/// Fetch a value from the database into cache, returning a reference-handle
/// asynchronously. The key is referenced directly to perform the query.
#[implement(super::Map)]
#[tracing::instrument(skip(self, key), fields(%self), level = "trace")]
pub fn get<K>(self: &Arc<Self>, key: &K) -> impl Future<Output = Result<Handle<'_>>> + Send
where
	K: AsRef<[u8]> + Debug + ?Sized,
{
	use crate::pool::{Cmd, Get};

	let cached = self.get_cached(key);
	if matches!(cached, Err(_) | Ok(Some(_))) {
		return future::ready(cached.map(|res| res.expect("Option is Some"))).boxed();
	}

	debug_assert!(matches!(cached, Ok(None)), "expected status Incomplete");
	let cmd = Cmd::Get(Get {
		map: self.clone(),
		key: key.as_ref().into(),
		res: None,
	});

	self.db.pool.execute(cmd).boxed()
}

#[implement(super::Map)]
#[tracing::instrument(skip(self, keys), name = "batch_blocking", level = "trace")]
pub(crate) fn get_batch_blocking<'a, I, K>(&self, keys: I) -> impl Iterator<Item = Result<Handle<'_>>> + Send
where
	I: Iterator<Item = &'a K> + ExactSizeIterator + Debug + Send,
	K: AsRef<[u8]> + Debug + Send + ?Sized + Sync + 'a,
{
	// Optimization can be `true` if key vector is pre-sorted **by the column
	// comparator**.
	const SORTED: bool = false;

	let read_options = &self.read_options;
	self.db
		.db
		.batched_multi_get_cf_opt(&self.cf(), keys, SORTED, read_options)
		.into_iter()
		.map(into_result_handle)
}

/// Fetch a value from the database into cache, returning a reference-handle.
/// The key is referenced directly to perform the query. This is a thread-
/// blocking call.
#[implement(super::Map)]
#[tracing::instrument(skip(self, key), name = "blocking", level = "trace")]
pub fn get_blocking<K>(&self, key: &K) -> Result<Handle<'_>>
where
	K: AsRef<[u8]> + ?Sized,
{
	let res = self
		.db
		.db
		.get_pinned_cf_opt(&self.cf(), key, &self.read_options);

	into_result_handle(res)
}

/// Fetch a value from the cache without I/O.
#[implement(super::Map)]
#[tracing::instrument(skip(self, key), name = "cache", level = "trace")]
pub(crate) fn get_cached<K>(&self, key: &K) -> Result<Option<Handle<'_>>>
where
	K: AsRef<[u8]> + Debug + ?Sized,
{
	let res = self
		.db
		.db
		.get_pinned_cf_opt(&self.cf(), key, &self.cache_read_options);

	match res {
		// cache hit; not found
		Ok(None) => Err!(Request(NotFound("Not found in database"))),

		// cache hit; value found
		Ok(Some(res)) => Ok(Some(Handle::from(res))),

		// cache miss; unknown
		Err(e) if is_incomplete(&e) => Ok(None),

		// some other error occurred
		Err(e) => or_else(e),
	}
}

fn into_result_handle(result: RocksdbResult<'_>) -> Result<Handle<'_>> {
	result
		.map_err(map_err)?
		.map(Handle::from)
		.ok_or(err!(Request(NotFound("Not found in database"))))
}
