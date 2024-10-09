use std::{convert::AsRef, fmt::Debug};

use conduit::{implement, Result};
use futures::{
	future,
	stream::{Stream, StreamExt},
	TryStreamExt,
};
use serde::{Deserialize, Serialize};

use crate::{keyval, keyval::Key, ser};

#[implement(super::Map)]
#[tracing::instrument(skip(self), fields(%self), level = "trace")]
pub fn rev_keys_prefix<'a, K, P>(&'a self, prefix: &P) -> impl Stream<Item = Result<Key<'_, K>>> + Send
where
	P: Serialize + ?Sized + Debug,
	K: Deserialize<'a> + Send,
{
	self.rev_keys_prefix_raw(prefix)
		.map(keyval::result_deserialize_key::<K>)
}

#[implement(super::Map)]
#[tracing::instrument(skip(self), fields(%self), level = "trace")]
pub fn rev_keys_prefix_raw<P>(&self, prefix: &P) -> impl Stream<Item = Result<Key<'_>>> + Send
where
	P: Serialize + ?Sized + Debug,
{
	let key = ser::serialize_to_vec(prefix).expect("failed to serialize query key");
	self.rev_raw_keys_from(&key)
		.try_take_while(move |k: &Key<'_>| future::ok(k.starts_with(&key)))
}

#[implement(super::Map)]
#[tracing::instrument(skip(self), fields(%self), level = "trace")]
pub fn rev_keys_raw_prefix<'a, K, P>(&'a self, prefix: &'a P) -> impl Stream<Item = Result<Key<'_, K>>> + Send + 'a
where
	P: AsRef<[u8]> + ?Sized + Debug + Sync + 'a,
	K: Deserialize<'a> + Send + 'a,
{
	self.rev_raw_keys_prefix(prefix)
		.map(keyval::result_deserialize_key::<K>)
}

#[implement(super::Map)]
#[tracing::instrument(skip(self), fields(%self), level = "trace")]
pub fn rev_raw_keys_prefix<'a, P>(&'a self, prefix: &'a P) -> impl Stream<Item = Result<Key<'_>>> + Send + 'a
where
	P: AsRef<[u8]> + ?Sized + Debug + Sync + 'a,
{
	self.rev_raw_keys_from(prefix)
		.try_take_while(|k: &Key<'_>| future::ok(k.starts_with(prefix.as_ref())))
}
