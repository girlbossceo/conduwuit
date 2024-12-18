use std::sync::Arc;

use conduwuit::{implement, Result};
use futures::{FutureExt, Stream, StreamExt, TryFutureExt, TryStreamExt};
use rocksdb::Direction;
use serde::Deserialize;
use tokio::task;

use super::rev_stream::is_cached;
use crate::{keyval, keyval::Key, stream};

#[implement(super::Map)]
pub fn rev_keys<'a, K>(self: &'a Arc<Self>) -> impl Stream<Item = Result<Key<'_, K>>> + Send
where
	K: Deserialize<'a> + Send,
{
	self.rev_raw_keys().map(keyval::result_deserialize_key::<K>)
}

#[implement(super::Map)]
#[tracing::instrument(skip(self), fields(%self), level = "trace")]
pub fn rev_raw_keys(self: &Arc<Self>) -> impl Stream<Item = Result<Key<'_>>> + Send {
	use crate::pool::Seek;

	let opts = super::iter_options_default();
	let state = stream::State::new(&self.db, &self.cf, opts);
	if is_cached(self) {
		let state = state.init_rev(None);
		return task::consume_budget()
			.map(move |()| stream::KeysRev::<'_>::from(state))
			.into_stream()
			.flatten()
			.boxed();
	}

	let seek = Seek {
		map: self.clone(),
		dir: Direction::Reverse,
		state: crate::pool::into_send_seek(state),
		key: None,
		res: None,
	};

	self.db
		.pool
		.execute_iter(seek)
		.ok_into::<stream::KeysRev<'_>>()
		.into_stream()
		.try_flatten()
		.boxed()
}
