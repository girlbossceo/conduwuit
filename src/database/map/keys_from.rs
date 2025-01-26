use std::{convert::AsRef, fmt::Debug, sync::Arc};

use conduwuit::{implement, Result};
use futures::{FutureExt, Stream, StreamExt, TryFutureExt, TryStreamExt};
use rocksdb::Direction;
use serde::{Deserialize, Serialize};

use super::stream_from::is_cached;
use crate::{
	keyval::{result_deserialize_key, serialize_key, Key},
	stream,
};

#[implement(super::Map)]
pub fn keys_from<'a, K, P>(
	self: &'a Arc<Self>,
	from: &P,
) -> impl Stream<Item = Result<Key<'_, K>>> + Send
where
	P: Serialize + ?Sized + Debug,
	K: Deserialize<'a> + Send,
{
	self.keys_from_raw(from).map(result_deserialize_key::<K>)
}

#[implement(super::Map)]
#[tracing::instrument(skip(self), level = "trace")]
pub fn keys_from_raw<P>(self: &Arc<Self>, from: &P) -> impl Stream<Item = Result<Key<'_>>> + Send
where
	P: Serialize + ?Sized + Debug,
{
	let key = serialize_key(from).expect("failed to serialize query key");
	self.raw_keys_from(&key)
}

#[implement(super::Map)]
pub fn keys_raw_from<'a, K, P>(
	self: &'a Arc<Self>,
	from: &P,
) -> impl Stream<Item = Result<Key<'_, K>>> + Send
where
	P: AsRef<[u8]> + ?Sized + Debug + Sync,
	K: Deserialize<'a> + Send,
{
	self.raw_keys_from(from).map(result_deserialize_key::<K>)
}

#[implement(super::Map)]
#[tracing::instrument(skip(self, from), fields(%self), level = "trace")]
pub fn raw_keys_from<P>(self: &Arc<Self>, from: &P) -> impl Stream<Item = Result<Key<'_>>> + Send
where
	P: AsRef<[u8]> + ?Sized + Debug,
{
	use crate::pool::Seek;

	let opts = super::iter_options_default(&self.db);
	let state = stream::State::new(self, opts);
	if is_cached(self, from) {
		return stream::Keys::<'_>::from(state.init_fwd(from.as_ref().into())).boxed();
	}

	let seek = Seek {
		map: self.clone(),
		dir: Direction::Forward,
		key: Some(from.as_ref().into()),
		state: crate::pool::into_send_seek(state),
		res: None,
	};

	self.db
		.pool
		.execute_iter(seek)
		.ok_into::<stream::Keys<'_>>()
		.into_stream()
		.try_flatten()
		.boxed()
}
