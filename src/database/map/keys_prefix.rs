use std::{convert::AsRef, fmt::Debug, sync::Arc};

use conduwuit::{implement, Result};
use futures::{future, Stream, StreamExt, TryStreamExt};
use serde::{Deserialize, Serialize};

use crate::keyval::{result_deserialize_key, serialize_key, Key};

#[implement(super::Map)]
pub fn keys_prefix<'a, K, P>(
	self: &'a Arc<Self>,
	prefix: &P,
) -> impl Stream<Item = Result<Key<'_, K>>> + Send
where
	P: Serialize + ?Sized + Debug,
	K: Deserialize<'a> + Send,
{
	self.keys_prefix_raw(prefix)
		.map(result_deserialize_key::<K>)
}

#[implement(super::Map)]
#[tracing::instrument(skip(self), level = "trace")]
pub fn keys_prefix_raw<P>(
	self: &Arc<Self>,
	prefix: &P,
) -> impl Stream<Item = Result<Key<'_>>> + Send
where
	P: Serialize + ?Sized + Debug,
{
	let key = serialize_key(prefix).expect("failed to serialize query key");
	self.raw_keys_from(&key)
		.try_take_while(move |k: &Key<'_>| future::ok(k.starts_with(&key)))
}

#[implement(super::Map)]
pub fn keys_raw_prefix<'a, K, P>(
	self: &'a Arc<Self>,
	prefix: &'a P,
) -> impl Stream<Item = Result<Key<'_, K>>> + Send + 'a
where
	P: AsRef<[u8]> + ?Sized + Debug + Sync + 'a,
	K: Deserialize<'a> + Send + 'a,
{
	self.raw_keys_prefix(prefix)
		.map(result_deserialize_key::<K>)
}

#[implement(super::Map)]
pub fn raw_keys_prefix<'a, P>(
	self: &'a Arc<Self>,
	prefix: &'a P,
) -> impl Stream<Item = Result<Key<'_>>> + Send + 'a
where
	P: AsRef<[u8]> + ?Sized + Debug + Sync + 'a,
{
	self.raw_keys_from(prefix)
		.try_take_while(|k: &Key<'_>| future::ok(k.starts_with(prefix.as_ref())))
}
