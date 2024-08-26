mod commands;

use clap::Subcommand;
use conduit::Result;
use ruma::{EventId, MxcUri, OwnedMxcUri, ServerName};

use crate::admin_command_dispatch;

#[admin_command_dispatch]
#[derive(Debug, Subcommand)]
pub(super) enum MediaCommand {
	/// - Deletes a single media file from our database and on the filesystem
	///   via a single MXC URL
	Delete {
		/// The MXC URL to delete
		#[arg(long)]
		mxc: Option<Box<MxcUri>>,

		/// - The message event ID which contains the media and thumbnail MXC
		///   URLs
		#[arg(long)]
		event_id: Option<Box<EventId>>,
	},

	/// - Deletes a codeblock list of MXC URLs from our database and on the
	///   filesystem
	DeleteList,

	/// - Deletes all remote media in the last X amount of time using filesystem
	///   metadata first created at date.
	DeletePastRemoteMedia {
		/// - The duration (at or after), e.g. "5m" to delete all media in the
		///   past 5 minutes
		duration: String,

		/// Continues deleting remote media if an undeletable object is found
		#[arg(short, long)]
		force: bool,
	},

	/// - Deletes all the local media from a local user on our server
	DeleteAllFromUser {
		username: String,

		/// Continues deleting media if an undeletable object is found
		#[arg(short, long)]
		force: bool,
	},

	/// - Deletes all remote media from the specified remote server
	DeleteAllFromServer {
		server_name: Box<ServerName>,

		/// Continues deleting media if an undeletable object is found
		#[arg(short, long)]
		force: bool,
	},

	GetFileInfo {
		/// The MXC URL to lookup info for.
		mxc: OwnedMxcUri,
	},
}
