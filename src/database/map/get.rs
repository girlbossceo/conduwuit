use std::{convert::AsRef, fmt::Debug, future::Future, io::Write};

use arrayvec::ArrayVec;
use conduit::{err, implement, Result};
use futures::future::ready;
use serde::Serialize;

use crate::{
	keyval::{OwnedKey, OwnedVal},
	ser,
	util::{map_err, or_else},
	Handle,
};

/// Fetch a value from the database into cache, returning a reference-handle
/// asynchronously. The key is serialized into an allocated buffer to perform
/// the query.
#[implement(super::Map)]
pub fn qry<K>(&self, key: &K) -> impl Future<Output = Result<Handle<'_>>> + Send
where
	K: Serialize + ?Sized + Debug,
{
	let mut buf = Vec::<u8>::with_capacity(64);
	self.bqry(key, &mut buf)
}

/// Fetch a value from the database into cache, returning a reference-handle
/// asynchronously. The key is serialized into a fixed-sized buffer to perform
/// the query. The maximum size is supplied as const generic parameter.
#[implement(super::Map)]
pub fn aqry<const MAX: usize, K>(&self, key: &K) -> impl Future<Output = Result<Handle<'_>>> + Send
where
	K: Serialize + ?Sized + Debug,
{
	let mut buf = ArrayVec::<u8, MAX>::new();
	self.bqry(key, &mut buf)
}

/// Fetch a value from the database into cache, returning a reference-handle
/// asynchronously. The key is serialized into a user-supplied Writer.
#[implement(super::Map)]
#[tracing::instrument(skip(self, buf), fields(%self), level = "trace")]
pub fn bqry<K, B>(&self, key: &K, buf: &mut B) -> impl Future<Output = Result<Handle<'_>>> + Send
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
pub fn get<K>(&self, key: &K) -> impl Future<Output = Result<Handle<'_>>> + Send
where
	K: AsRef<[u8]> + ?Sized + Debug,
{
	ready(self.get_blocking(key))
}

/// Fetch a value from the database into cache, returning a reference-handle.
/// The key is referenced directly to perform the query. This is a thread-
/// blocking call.
#[implement(super::Map)]
#[tracing::instrument(skip(self, key), fields(%self), level = "trace")]
pub fn get_blocking<K>(&self, key: &K) -> Result<Handle<'_>>
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

#[implement(super::Map)]
#[tracing::instrument(skip(self, keys), fields(%self), level = "trace")]
pub fn get_batch_blocking<'a, I, K>(&self, keys: I) -> Vec<Option<OwnedVal>>
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
