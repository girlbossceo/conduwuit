use std::{convert::AsRef, fmt::Debug};

use conduit::{implement, Result};
use futures::{Stream, StreamExt};
use serde::{Deserialize, Serialize};

use crate::{keyval, keyval::Key, ser, stream};

#[implement(super::Map)]
#[tracing::instrument(skip(self), fields(%self), level = "trace")]
pub fn keys_from<'a, K, P>(&'a self, from: &P) -> impl Stream<Item = Result<Key<'_, K>>> + Send
where
	P: Serialize + ?Sized + Debug,
	K: Deserialize<'a> + Send,
{
	self.keys_raw_from(from)
		.map(keyval::result_deserialize_key::<K>)
}

#[implement(super::Map)]
#[tracing::instrument(skip(self), fields(%self), level = "trace")]
pub fn keys_raw_from<P>(&self, from: &P) -> impl Stream<Item = Result<Key<'_>>> + Send
where
	P: Serialize + ?Sized + Debug,
{
	let key = ser::serialize_to_vec(from).expect("failed to serialize query key");
	self.raw_keys_from(&key)
}

#[implement(super::Map)]
#[tracing::instrument(skip(self), fields(%self), level = "trace")]
pub fn keys_from_raw<'a, K, P>(&'a self, from: &P) -> impl Stream<Item = Result<Key<'_, K>>> + Send
where
	P: AsRef<[u8]> + ?Sized + Debug + Sync,
	K: Deserialize<'a> + Send,
{
	self.raw_keys_from(from)
		.map(keyval::result_deserialize_key::<K>)
}

#[implement(super::Map)]
#[tracing::instrument(skip(self), fields(%self), level = "trace")]
pub fn raw_keys_from<P>(&self, from: &P) -> impl Stream<Item = Result<Key<'_>>> + Send
where
	P: AsRef<[u8]> + ?Sized + Debug,
{
	let opts = super::read_options_default();
	stream::Keys::new(&self.db, &self.cf, opts, Some(from.as_ref()))
}
