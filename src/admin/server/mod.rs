mod commands;

use clap::Subcommand;
use conduit::Result;
use ruma::events::room::message::RoomMessageEventContent;

use self::commands::*;

#[derive(Debug, Subcommand)]
pub(super) enum ServerCommand {
	/// - Time elapsed since startup
	Uptime,

	/// - Show configuration values
	ShowConfig,

	/// - Print database memory usage statistics
	MemoryUsage,

	/// - Clears all of Conduwuit's caches
	ClearCaches,

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
	Restart {
		#[arg(short, long)]
		force: bool,
	},

	/// - Shutdown the server
	Shutdown,
}

pub(super) async fn process(command: ServerCommand, body: Vec<&str>) -> Result<RoomMessageEventContent> {
	Ok(match command {
		ServerCommand::Uptime => uptime(body).await?,
		ServerCommand::ShowConfig => show_config(body).await?,
		ServerCommand::MemoryUsage => memory_usage(body).await?,
		ServerCommand::ClearCaches => clear_caches(body).await?,
		ServerCommand::ListBackups => list_backups(body).await?,
		ServerCommand::BackupDatabase => backup_database(body).await?,
		ServerCommand::ListDatabaseFiles => list_database_files(body).await?,
		ServerCommand::AdminNotice {
			message,
		} => admin_notice(body, message).await?,
		#[cfg(conduit_mods)]
		ServerCommand::Reload => reload(body).await?,
		#[cfg(unix)]
		ServerCommand::Restart {
			force,
		} => restart(body, force).await?,
		ServerCommand::Shutdown => shutdown(body).await?,
	})
}
