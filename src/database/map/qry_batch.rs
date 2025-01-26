use std::{fmt::Debug, sync::Arc};

use conduwuit::{
	implement,
	utils::{
		stream::{automatic_amplification, automatic_width, WidebandExt},
		IterStream,
	},
	Result,
};
use futures::{Stream, StreamExt, TryStreamExt};
use serde::Serialize;

use crate::{keyval::KeyBuf, ser, Handle};

pub trait Qry<'a, K, S>
where
	S: Stream<Item = K> + Send + 'a,
	K: Serialize + Debug,
{
	fn qry(self, map: &'a Arc<super::Map>) -> impl Stream<Item = Result<Handle<'_>>> + Send + 'a;
}

impl<'a, K, S> Qry<'a, K, S> for S
where
	Self: 'a,
	S: Stream<Item = K> + Send + 'a,
	K: Serialize + Debug + 'a,
{
	#[inline]
	fn qry(self, map: &'a Arc<super::Map>) -> impl Stream<Item = Result<Handle<'_>>> + Send + 'a {
		map.qry_batch(self)
	}
}

#[implement(super::Map)]
#[tracing::instrument(skip(self, keys), level = "trace")]
pub(crate) fn qry_batch<'a, S, K>(
	self: &'a Arc<Self>,
	keys: S,
) -> impl Stream<Item = Result<Handle<'_>>> + Send + 'a
where
	S: Stream<Item = K> + Send + 'a,
	K: Serialize + Debug + 'a,
{
	use crate::pool::Get;

	keys.ready_chunks(automatic_amplification())
		.widen_then(automatic_width(), |chunk| {
			let keys = chunk
				.iter()
				.map(ser::serialize_to::<KeyBuf, _>)
				.map(|result| result.expect("failed to serialize query key"))
				.map(Into::into)
				.collect();

			self.db
				.pool
				.execute_get(Get { map: self.clone(), key: keys, res: None })
		})
		.map_ok(|results| results.into_iter().stream())
		.try_flatten()
}
