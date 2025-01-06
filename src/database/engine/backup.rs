use std::fmt::Write;

use conduwuit::{error, implement, info, utils::time::rfc2822_from_seconds, warn, Result};
use rocksdb::backup::{BackupEngine, BackupEngineOptions};

use super::Engine;
use crate::{or_else, util::map_err};

#[implement(Engine)]
#[tracing::instrument(skip(self))]
pub fn backup(&self) -> Result {
	let server = &self.ctx.server;
	let config = &server.config;
	let path = config.database_backup_path.as_ref();
	if path.is_none() || path.is_some_and(|path| path.as_os_str().is_empty()) {
		return Ok(());
	}

	let options =
		BackupEngineOptions::new(path.expect("valid database backup path")).map_err(map_err)?;
	let mut engine = BackupEngine::open(&options, &*self.ctx.env.lock()?).map_err(map_err)?;
	if config.database_backups_to_keep > 0 {
		let flush = !self.is_read_only();
		engine
			.create_new_backup_flush(&self.db, flush)
			.map_err(map_err)?;

		let engine_info = engine.get_backup_info();
		let info = &engine_info.last().expect("backup engine info is not empty");
		info!(
			"Created database backup #{} using {} bytes in {} files",
			info.backup_id, info.size, info.num_files,
		);
	}

	if config.database_backups_to_keep >= 0 {
		let keep = u32::try_from(config.database_backups_to_keep)?;
		if let Err(e) = engine.purge_old_backups(keep.try_into()?) {
			error!("Failed to purge old backup: {e:?}");
		}
	}

	Ok(())
}

#[implement(Engine)]
pub fn backup_list(&self) -> Result<String> {
	let server = &self.ctx.server;
	let config = &server.config;
	let path = config.database_backup_path.as_ref();
	if path.is_none() || path.is_some_and(|path| path.as_os_str().is_empty()) {
		return Ok("Configure database_backup_path to enable backups, or the path specified is \
		           not valid"
			.to_owned());
	}

	let mut res = String::new();
	let options =
		BackupEngineOptions::new(path.expect("valid database backup path")).or_else(or_else)?;
	let engine = BackupEngine::open(&options, &*self.ctx.env.lock()?).or_else(or_else)?;
	for info in engine.get_backup_info() {
		writeln!(
			res,
			"#{} {}: {} bytes, {} files",
			info.backup_id,
			rfc2822_from_seconds(info.timestamp),
			info.size,
			info.num_files,
		)?;
	}

	Ok(res)
}
