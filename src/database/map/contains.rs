use std::{convert::AsRef, fmt::Debug, future::Future, io::Write};

use arrayvec::ArrayVec;
use conduit::{implement, utils::TryFutureExtExt, Err, Result};
use futures::future::ready;
use serde::Serialize;

use crate::{ser, util};

/// Returns true if the map contains the key.
/// - key is serialized into allocated buffer
/// - harder errors may not be reported
#[implement(super::Map)]
pub fn contains<K>(&self, key: &K) -> impl Future<Output = bool> + Send
where
	K: Serialize + ?Sized + Debug,
{
	let mut buf = Vec::<u8>::with_capacity(64);
	self.bcontains(key, &mut buf)
}

/// Returns true if the map contains the key.
/// - key is serialized into stack-buffer
/// - harder errors will panic
#[implement(super::Map)]
pub fn acontains<const MAX: usize, K>(&self, key: &K) -> impl Future<Output = bool> + Send
where
	K: Serialize + ?Sized + Debug,
{
	let mut buf = ArrayVec::<u8, MAX>::new();
	self.bcontains(key, &mut buf)
}

/// Returns true if the map contains the key.
/// - key is serialized into provided buffer
/// - harder errors will panic
#[implement(super::Map)]
#[tracing::instrument(skip(self, buf), fields(%self), level = "trace")]
pub fn bcontains<K, B>(&self, key: &K, buf: &mut B) -> impl Future<Output = bool> + Send
where
	K: Serialize + ?Sized + Debug,
	B: Write + AsRef<[u8]>,
{
	let key = ser::serialize(buf, key).expect("failed to serialize query key");
	self.exists(key).is_ok()
}

/// Returns Ok if the map contains the key.
/// - key is raw
#[implement(super::Map)]
pub fn exists<K>(&self, key: &K) -> impl Future<Output = Result<()>> + Send
where
	K: AsRef<[u8]> + ?Sized + Debug,
{
	ready(self.exists_blocking(key))
}

/// Returns Ok if the map contains the key; NotFound otherwise. Harder errors
/// may not always be reported properly.
#[implement(super::Map)]
#[tracing::instrument(skip(self, key), fields(%self), level = "trace")]
pub fn exists_blocking<K>(&self, key: &K) -> Result<()>
where
	K: AsRef<[u8]> + ?Sized + Debug,
{
	if self.maybe_exists_blocking(key)
		&& self
			.db
			.db
			.get_pinned_cf_opt(&self.cf(), key, &self.read_options)
			.map_err(util::map_err)?
			.is_some()
	{
		Ok(())
	} else {
		Err!(Request(NotFound("Not found in database")))
	}
}

#[implement(super::Map)]
fn maybe_exists_blocking<K>(&self, key: &K) -> bool
where
	K: AsRef<[u8]> + ?Sized,
{
	self.db
		.db
		.key_may_exist_cf_opt(&self.cf(), key, &self.read_options)
}
