use std::sync::Arc;

use conduwuit::{
	Result, implement,
	utils::stream::{ReadyExt, TryIgnore},
};
use futures::{Stream, TryStreamExt};

use crate::keyval::Key;

/// Delete all data stored in this map. !!! USE WITH CAUTION !!!
///
/// See for_clear() with additional details.
#[implement(super::Map)]
#[tracing::instrument(level = "trace")]
pub async fn clear(self: &Arc<Self>) {
	self.for_clear().ignore_err().ready_for_each(|_| ()).await;
}

/// Delete all data stored in this map. !!! USE WITH CAUTION !!!
///
/// Provides stream of keys undergoing deletion along with any errors.
///
/// Note this operation applies to a snapshot of the data when invoked.
/// Additional data written during or after this call may be missed.
#[implement(super::Map)]
#[tracing::instrument(level = "trace")]
pub fn for_clear(self: &Arc<Self>) -> impl Stream<Item = Result<Key<'_>>> + Send {
	self.raw_keys().inspect_ok(|key| self.remove(key))
}
