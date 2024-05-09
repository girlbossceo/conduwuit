use clap::Subcommand;
use ruma::{events::room::message::RoomMessageEventContent, EventId, MxcUri};

use self::media_commands::{delete, delete_list, delete_past_remote_media};
use crate::Result;

pub(crate) mod media_commands;

#[cfg_attr(test, derive(Debug))]
#[derive(Subcommand)]
pub(crate) enum MediaCommand {
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
	},
}

pub(crate) async fn process(command: MediaCommand, body: Vec<&str>) -> Result<RoomMessageEventContent> {
	Ok(match command {
		MediaCommand::Delete {
			mxc,
			event_id,
		} => delete(body, mxc, event_id).await?,
		MediaCommand::DeleteList => delete_list(body).await?,
		MediaCommand::DeletePastRemoteMedia {
			duration,
		} => delete_past_remote_media(body, duration).await?,
	})
}
