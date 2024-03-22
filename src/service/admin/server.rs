use clap::Subcommand;
use ruma::events::room::message::RoomMessageEventContent;

use crate::{services, Result};

#[cfg_attr(test, derive(Debug))]
#[derive(Subcommand)]
pub(crate) enum ServerCommand {
	/// - Show configuration values
	ShowConfig,

	/// - Print database memory usage statistics
	MemoryUsage,

	/// - Clears all of Conduit's database caches with index smaller than the
	///   amount
	ClearDatabaseCaches {
		amount: u32,
	},

	/// - Clears all of Conduit's service caches with index smaller than the
	///   amount
	ClearServiceCaches {
		amount: u32,
	},

	/// - Performs an online backup of the database (only available for RocksDB
	///   at the moment)
	BackupDatabase,

	/// - List database backups
	ListBackups,

	/// - List database files
	ListDatabaseFiles,
}

pub(crate) async fn process(command: ServerCommand, _body: Vec<&str>) -> Result<RoomMessageEventContent> {
	match command {
		ServerCommand::ShowConfig => {
			// Construct and send the response
			Ok(RoomMessageEventContent::text_plain(format!("{}", services().globals.config)))
		},
		ServerCommand::MemoryUsage => {
			let response1 = services().memory_usage().await;
			let response2 = services().globals.db.memory_usage();

			Ok(RoomMessageEventContent::text_plain(format!(
				"Services:\n{response1}\n\nDatabase:\n{response2}"
			)))
		},
		ServerCommand::ClearDatabaseCaches {
			amount,
		} => {
			services().globals.db.clear_caches(amount);

			Ok(RoomMessageEventContent::text_plain("Done."))
		},
		ServerCommand::ClearServiceCaches {
			amount,
		} => {
			services().clear_caches(amount).await;

			Ok(RoomMessageEventContent::text_plain("Done."))
		},
		ServerCommand::ListBackups => {
			let result = services().globals.db.backup_list()?;

			if result.is_empty() {
				Ok(RoomMessageEventContent::text_plain("No backups found."))
			} else {
				Ok(RoomMessageEventContent::text_plain(result))
			}
		},
		ServerCommand::BackupDatabase => {
			if !cfg!(feature = "rocksdb") {
				return Ok(RoomMessageEventContent::text_plain(
					"Only RocksDB supports online backups in conduwuit.",
				));
			}

			let mut result = tokio::task::spawn_blocking(move || match services().globals.db.backup() {
				Ok(()) => String::new(),
				Err(e) => (*e).to_string(),
			})
			.await
			.unwrap();

			if result.is_empty() {
				result = services().globals.db.backup_list()?;
			}

			Ok(RoomMessageEventContent::text_plain(&result))
		},
		ServerCommand::ListDatabaseFiles => {
			if !cfg!(feature = "rocksdb") {
				return Ok(RoomMessageEventContent::text_plain(
					"Only RocksDB supports listing files in conduwuit.",
				));
			}

			let result = services().globals.db.file_list()?;
			Ok(RoomMessageEventContent::notice_html(String::new(), result))
		},
	}
}
