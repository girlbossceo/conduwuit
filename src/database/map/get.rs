use std::{convert::AsRef, fmt::Debug, io::Write, sync::Arc};

use arrayvec::ArrayVec;
use conduwuit::{err, implement, utils::result::MapExpect, Err, Result};
use futures::{future::ready, Future, FutureExt, TryFutureExt};
use serde::Serialize;
use tokio::task;

use crate::{
	keyval::KeyBuf,
	ser,
	util::{is_incomplete, map_err, or_else},
	Handle,
};

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
pub fn aqry<const MAX: usize, K>(
	self: &Arc<Self>,
	key: &K,
) -> impl Future<Output = Result<Handle<'_>>> + Send
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
pub fn bqry<K, B>(
	self: &Arc<Self>,
	key: &K,
	buf: &mut B,
) -> impl Future<Output = Result<Handle<'_>>> + Send
where
	K: Serialize + ?Sized + Debug,
	B: Write + AsRef<[u8]>,
{
	let key = ser::serialize(buf, key).expect("failed to serialize query key");
	self.get(key)
}

/// Fetch a value from the database into cache, returning a reference-handle
/// asynchronously. The key is referenced directly to perform the query.
#[implement(super::Map)]
#[tracing::instrument(skip(self, key), fields(%self), level = "trace")]
pub fn get<K>(self: &Arc<Self>, key: &K) -> impl Future<Output = Result<Handle<'_>>> + Send
where
	K: AsRef<[u8]> + Debug + ?Sized,
{
	use crate::pool::Get;

	let cached = self.get_cached(key);
	if matches!(cached, Err(_) | Ok(Some(_))) {
		return task::consume_budget()
			.map(move |()| cached.map_expect("data found in cache"))
			.boxed();
	}

	debug_assert!(matches!(cached, Ok(None)), "expected status Incomplete");
	let cmd = Get {
		map: self.clone(),
		key: [key.as_ref().into()].into(),
		res: None,
	};

	self.db
		.pool
		.execute_get(cmd)
		.and_then(|mut res| ready(res.remove(0)))
		.boxed()
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
	self.db
		.db
		.get_pinned_cf_opt(&self.cf(), key, &self.read_options)
		.map_err(map_err)?
		.map(Handle::from)
		.ok_or(err!(Request(NotFound("Not found in database"))))
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
		| Ok(None) => Err!(Request(NotFound("Not found in database"))),

		// cache hit; value found
		| Ok(Some(res)) => Ok(Some(Handle::from(res))),

		// cache miss; unknown
		| Err(e) if is_incomplete(&e) => Ok(None),

		// some other error occurred
		| Err(e) => or_else(e),
	}
}
