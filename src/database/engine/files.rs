use conduwuit::{implement, Result};
use rocksdb::LiveFile as SstFile;

use super::Engine;
use crate::util::map_err;

#[implement(Engine)]
pub fn file_list(&self) -> impl Iterator<Item = Result<SstFile>> + Send {
	self.db
		.live_files()
		.map_err(map_err)
		.into_iter()
		.flat_map(Vec::into_iter)
		.map(Ok)
}
