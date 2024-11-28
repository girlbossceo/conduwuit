use std::{convert::AsRef, fmt::Debug};

use conduit::{implement, Result};
use futures::{Stream, StreamExt};
use serde::{Deserialize, Serialize};

use crate::{
	keyval::{result_deserialize_key, serialize_key, Key},
	stream,
};

#[implement(super::Map)]
pub fn rev_keys_from<'a, K, P>(&'a self, from: &P) -> impl Stream<Item = Result<Key<'_, K>>> + Send
where
	P: Serialize + ?Sized + Debug,
	K: Deserialize<'a> + Send,
{
	self.rev_keys_from_raw(from)
		.map(result_deserialize_key::<K>)
}

#[implement(super::Map)]
#[tracing::instrument(skip(self), level = "trace")]
pub fn rev_keys_from_raw<P>(&self, from: &P) -> impl Stream<Item = Result<Key<'_>>> + Send
where
	P: Serialize + ?Sized + Debug,
{
	let key = serialize_key(from).expect("failed to serialize query key");
	self.rev_raw_keys_from(&key)
}

#[implement(super::Map)]
pub fn rev_keys_raw_from<'a, K, P>(&'a self, from: &P) -> impl Stream<Item = Result<Key<'_, K>>> + Send
where
	P: AsRef<[u8]> + ?Sized + Debug + Sync,
	K: Deserialize<'a> + Send,
{
	self.rev_raw_keys_from(from)
		.map(result_deserialize_key::<K>)
}

#[implement(super::Map)]
#[tracing::instrument(skip(self, from), fields(%self), level = "trace")]
pub fn rev_raw_keys_from<P>(&self, from: &P) -> impl Stream<Item = Result<Key<'_>>> + Send
where
	P: AsRef<[u8]> + ?Sized + Debug,
{
	let opts = super::read_options_default();
	stream::KeysRev::new(&self.db, &self.cf, opts, Some(from.as_ref()))
}
