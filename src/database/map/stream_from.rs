use std::{convert::AsRef, fmt::Debug};

use conduit::{implement, Result};
use futures::stream::{Stream, StreamExt};
use serde::{Deserialize, Serialize};

use crate::{keyval, keyval::KeyVal, ser, stream};

/// Iterate key-value entries in the map starting from lower-bound.
///
/// - Query is serialized
/// - Result is deserialized
#[implement(super::Map)]
#[tracing::instrument(skip(self), fields(%self), level = "trace")]
pub fn stream_from<'a, K, V, P>(&'a self, from: &P) -> impl Stream<Item = Result<KeyVal<'_, K, V>>> + Send
where
	P: Serialize + ?Sized + Debug,
	K: Deserialize<'a> + Send,
	V: Deserialize<'a> + Send,
{
	self.stream_raw_from(from)
		.map(keyval::result_deserialize::<K, V>)
}

/// Iterate key-value entries in the map starting from lower-bound.
///
/// - Query is serialized
/// - Result is raw
#[implement(super::Map)]
#[tracing::instrument(skip(self), fields(%self), level = "trace")]
pub fn stream_raw_from<P>(&self, from: &P) -> impl Stream<Item = Result<KeyVal<'_>>> + Send
where
	P: Serialize + ?Sized + Debug,
{
	let key = ser::serialize_to_vec(from).expect("failed to serialize query key");
	self.raw_stream_from(&key)
}

/// Iterate key-value entries in the map starting from lower-bound.
///
/// - Query is raw
/// - Result is deserialized
#[implement(super::Map)]
#[tracing::instrument(skip(self), fields(%self), level = "trace")]
pub fn stream_from_raw<'a, K, V, P>(&'a self, from: &P) -> impl Stream<Item = Result<KeyVal<'_, K, V>>> + Send
where
	P: AsRef<[u8]> + ?Sized + Debug + Sync,
	K: Deserialize<'a> + Send,
	V: Deserialize<'a> + Send,
{
	self.raw_stream_from(from)
		.map(keyval::result_deserialize::<K, V>)
}

/// Iterate key-value entries in the map starting from lower-bound.
///
/// - Query is raw
/// - Result is raw
#[implement(super::Map)]
#[tracing::instrument(skip(self), fields(%self), level = "trace")]
pub fn raw_stream_from<P>(&self, from: &P) -> impl Stream<Item = Result<KeyVal<'_>>> + Send
where
	P: AsRef<[u8]> + ?Sized + Debug,
{
	let opts = super::read_options_default();
	stream::Items::new(&self.db, &self.cf, opts, Some(from.as_ref()))
}
