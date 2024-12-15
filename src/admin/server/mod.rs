mod commands;

use clap::Subcommand;
use conduwuit::Result;

use crate::admin_command_dispatch;

#[admin_command_dispatch]
#[derive(Debug, Subcommand)]
pub(super) enum ServerCommand {
	/// - Time elapsed since startup
	Uptime,

	/// - Show configuration values
	ShowConfig,

	/// - List the features built into the server
	ListFeatures {
		#[arg(short, long)]
		available: bool,

		#[arg(short, long)]
		enabled: bool,

		#[arg(short, long)]
		comma: bool,
	},

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

	/// - Hot-reload the server
	#[clap(alias = "reload")]
	ReloadMods,

	#[cfg(unix)]
	/// - Restart the server
	Restart {
		#[arg(short, long)]
		force: bool,
	},

	/// - Shutdown the server
	Shutdown,
}
