use conduit::{implement, Result};
use futures::{Stream, StreamExt};
use serde::Deserialize;

use crate::{keyval, keyval::Key, stream};

#[implement(super::Map)]
#[tracing::instrument(skip(self), fields(%self), level = "trace")]
pub fn rev_keys<'a, K>(&'a self) -> impl Stream<Item = Result<Key<'_, K>>> + Send
where
	K: Deserialize<'a> + Send,
{
	self.rev_raw_keys().map(keyval::result_deserialize_key::<K>)
}

#[implement(super::Map)]
#[tracing::instrument(skip(self), fields(%self), level = "trace")]
pub fn rev_raw_keys(&self) -> impl Stream<Item = Result<Key<'_>>> + Send {
	let opts = super::read_options_default();
	stream::KeysRev::new(&self.db, &self.cf, opts, None)
}
