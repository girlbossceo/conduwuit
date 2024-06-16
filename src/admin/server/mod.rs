pub(crate) mod server_commands;

use clap::Subcommand;
use ruma::events::room::message::RoomMessageEventContent;

use self::server_commands::*;
use crate::Result;

#[cfg_attr(test, derive(Debug))]
#[derive(Subcommand)]
pub(crate) enum ServerCommand {
	/// - Time elapsed since startup
	Uptime,

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

	/// - Send a message to the admin room.
	AdminNotice {
		message: Vec<String>,
	},

	#[cfg(conduit_mods)]
	/// - Hot-reload the server
	Reload,

	#[cfg(unix)]
	/// - Restart the server
	Restart,

	/// - Shutdown the server
	Shutdown,
}

pub(crate) async fn process(command: ServerCommand, body: Vec<&str>) -> Result<RoomMessageEventContent> {
	Ok(match command {
		ServerCommand::Uptime => uptime(body).await?,
		ServerCommand::ShowConfig => show_config(body).await?,
		ServerCommand::MemoryUsage => memory_usage(body).await?,
		ServerCommand::ClearDatabaseCaches {
			amount,
		} => clear_database_caches(body, amount).await?,
		ServerCommand::ClearServiceCaches {
			amount,
		} => clear_service_caches(body, amount).await?,
		ServerCommand::ListBackups => list_backups(body).await?,
		ServerCommand::BackupDatabase => backup_database(body).await?,
		ServerCommand::ListDatabaseFiles => list_database_files(body).await?,
		ServerCommand::AdminNotice {
			message,
		} => admin_notice(body, message).await?,
		#[cfg(conduit_mods)]
		ServerCommand::Reload => reload(body).await?,
		#[cfg(unix)]
		ServerCommand::Restart => restart(body).await?,
		ServerCommand::Shutdown => shutdown(body).await?,
	})
}
