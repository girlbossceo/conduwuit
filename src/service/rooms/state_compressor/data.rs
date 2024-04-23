use std::{collections::HashSet, sync::Arc};

use super::CompressedStateEvent;
use crate::Result;

pub(crate) struct StateDiff {
	pub(crate) parent: Option<u64>,
	pub(crate) added: Arc<HashSet<CompressedStateEvent>>,
	pub(crate) removed: Arc<HashSet<CompressedStateEvent>>,
}

pub(crate) trait Data: Send + Sync {
	fn get_statediff(&self, shortstatehash: u64) -> Result<StateDiff>;
	fn save_statediff(&self, shortstatehash: u64, diff: StateDiff) -> Result<()>;
}
