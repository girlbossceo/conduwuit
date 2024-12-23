use std::{convert::AsRef, fmt::Debug, sync::Arc};

use conduwuit::{
	err, implement,
	utils::{stream::automatic_width, IterStream},
	Result,
};
use futures::{Stream, StreamExt};
use serde::Serialize;

use crate::{util::map_err, Handle};

#[implement(super::Map)]
#[tracing::instrument(skip(self, keys), level = "trace")]
pub fn aqry_batch<'b, 'a: 'b, const MAX: usize, I, K>(
	self: &'a Arc<Self>,
	keys: I,
) -> impl Stream<Item = Result<Handle<'b>>> + Send + 'a
where
	I: Iterator<Item = &'b K> + Send + 'a,
	K: Serialize + ?Sized + Debug + 'b,
{
	keys.stream()
		.map(move |key| self.aqry::<MAX, _>(&key))
		.buffered(automatic_width())
}

#[implement(super::Map)]
#[tracing::instrument(skip(self, keys), level = "trace")]
pub fn get_batch<'a, I, K>(
	self: &'a Arc<Self>,
	keys: I,
) -> impl Stream<Item = Result<Handle<'_>>> + Send + 'a
where
	I: Iterator<Item = &'a K> + Debug + Send + 'a,
	K: AsRef<[u8]> + Debug + Send + ?Sized + Sync + 'a,
{
	keys.stream()
		.map(move |key| self.get(key))
		.buffered(automatic_width())
}

#[implement(super::Map)]
#[tracing::instrument(name = "batch_blocking", level = "trace", skip_all)]
pub(crate) fn get_batch_blocking<'a, I, K>(
	&self,
	keys: I,
) -> impl Iterator<Item = Result<Handle<'_>>> + Send
where
	I: Iterator<Item = &'a K> + ExactSizeIterator + Debug + Send,
	K: AsRef<[u8]> + Debug + Send + ?Sized + Sync + 'a,
{
	// Optimization can be `true` if key vector is pre-sorted **by the column
	// comparator**.
	const SORTED: bool = false;

	let read_options = &self.read_options;
	self.db
		.db
		.batched_multi_get_cf_opt(&self.cf(), keys, SORTED, read_options)
		.into_iter()
		.map(|result| {
			result
				.map_err(map_err)?
				.map(Handle::from)
				.ok_or(err!(Request(NotFound("Not found in database"))))
		})
}
