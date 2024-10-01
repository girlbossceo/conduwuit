mod commands;

use clap::Subcommand;
use conduit::Result;
use ruma::{EventId, MxcUri, OwnedMxcUri, OwnedServerName, ServerName};

use crate::admin_command_dispatch;

#[admin_command_dispatch]
#[derive(Debug, Subcommand)]
pub(super) enum MediaCommand {
	/// - Deletes a single media file from our database and on the filesystem
	///   via a single MXC URL or event ID (not redacted)
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
	///   filesystem. This will always ignore errors.
	DeleteList,

	/// - Deletes all remote media in the last/after "X" time using filesystem
	///   metadata first created at date, or fallback to last modified date.
	///   This will always ignore errors by default.
	///
	/// Synapse
	DeletePastRemoteMedia {
		/// - The duration (at or after/before), e.g. "5m" to delete all media
		///   in the past or up to 5 minutes
		duration: String,

		#[arg(long, short)]
		before: bool,

		#[arg(long, short)]
		after: bool,

		/// Long argument to delete local media
		#[arg(long)]
		yes_i_want_to_delete_local_media: bool,
	},

	/// - Deletes all the local media from a local user on our server. This will
	///   always ignore errors by default.
	DeleteAllFromUser {
		username: String,
	},

	/// - Deletes all remote media from the specified remote server. This will
	///   always ignore errors by default.
	DeleteAllFromServer {
		server_name: Box<ServerName>,

		/// Long argument to delete local media
		#[arg(long)]
		yes_i_want_to_delete_local_media: bool,
	},

	GetFileInfo {
		/// The MXC URL to lookup info for.
		mxc: OwnedMxcUri,
	},

	GetRemoteFile {
		/// The MXC URL to fetch
		mxc: OwnedMxcUri,

		#[arg(short, long)]
		server: Option<OwnedServerName>,

		#[arg(short, long, default_value("10000"))]
		timeout: u32,
	},

	GetRemoteThumbnail {
		/// The MXC URL to fetch
		mxc: OwnedMxcUri,

		#[arg(short, long)]
		server: Option<OwnedServerName>,

		#[arg(short, long, default_value("10000"))]
		timeout: u32,

		#[arg(short, long, default_value("800"))]
		width: u32,

		#[arg(short, long, default_value("800"))]
		height: u32,
	},
}
