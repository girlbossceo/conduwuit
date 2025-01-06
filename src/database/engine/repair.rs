use std::path::PathBuf;

use conduwuit::{info, warn, Err, Result};
use rocksdb::Options;

use super::Db;

pub(crate) fn repair(db_opts: &Options, path: &PathBuf) -> Result {
	warn!("Starting database repair. This may take a long time...");
	match Db::repair(db_opts, path) {
		| Ok(()) => info!("Database repair successful."),
		| Err(e) => return Err!("Repair failed: {e:?}"),
	}

	Ok(())
}
