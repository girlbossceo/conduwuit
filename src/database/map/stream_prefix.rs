use std::{convert::AsRef, fmt::Debug};

use conduit::{implement, Result};
use futures::{
	future,
	stream::{Stream, StreamExt},
	TryStreamExt,
};
use serde::{Deserialize, Serialize};

use crate::{keyval, keyval::KeyVal, ser};

/// Iterate key-value entries in the map where the key matches a prefix.
///
/// - Query is serialized
/// - Result is deserialized
#[implement(super::Map)]
#[tracing::instrument(skip(self), fields(%self), level = "trace")]
pub fn stream_prefix<'a, K, V, P>(&'a self, prefix: &P) -> impl Stream<Item = Result<KeyVal<'_, K, V>>> + Send
where
	P: Serialize + ?Sized + Debug,
	K: Deserialize<'a> + Send,
	V: Deserialize<'a> + Send,
{
	self.stream_raw_prefix(prefix)
		.map(keyval::result_deserialize::<K, V>)
}

/// Iterate key-value entries in the map where the key matches a prefix.
///
/// - Query is serialized
/// - Result is raw
#[implement(super::Map)]
#[tracing::instrument(skip(self), fields(%self), level = "trace")]
pub fn stream_raw_prefix<P>(&self, prefix: &P) -> impl Stream<Item = Result<KeyVal<'_>>> + Send
where
	P: Serialize + ?Sized + Debug,
{
	let key = ser::serialize_to_vec(prefix).expect("failed to serialize query key");
	self.raw_stream_from(&key)
		.try_take_while(move |(k, _): &KeyVal<'_>| future::ok(k.starts_with(&key)))
}

/// Iterate key-value entries in the map where the key matches a prefix.
///
/// - Query is raw
/// - Result is deserialized
#[implement(super::Map)]
#[tracing::instrument(skip(self), fields(%self), level = "trace")]
pub fn stream_prefix_raw<'a, K, V, P>(
	&'a self, prefix: &'a P,
) -> impl Stream<Item = Result<KeyVal<'_, K, V>>> + Send + 'a
where
	P: AsRef<[u8]> + ?Sized + Debug + Sync + 'a,
	K: Deserialize<'a> + Send + 'a,
	V: Deserialize<'a> + Send + 'a,
{
	self.raw_stream_prefix(prefix)
		.map(keyval::result_deserialize::<K, V>)
}

/// Iterate key-value entries in the map where the key matches a prefix.
///
/// - Query is raw
/// - Result is raw
#[implement(super::Map)]
#[tracing::instrument(skip(self), fields(%self), level = "trace")]
pub fn raw_stream_prefix<'a, P>(&'a self, prefix: &'a P) -> impl Stream<Item = Result<KeyVal<'_>>> + Send + 'a
where
	P: AsRef<[u8]> + ?Sized + Debug + Sync + 'a,
{
	self.raw_stream_from(prefix)
		.try_take_while(|(k, _): &KeyVal<'_>| future::ok(k.starts_with(prefix.as_ref())))
}
