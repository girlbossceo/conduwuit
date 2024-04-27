use clap::Subcommand;
use ruma::{events::room::message::RoomMessageEventContent, RoomId, ServerName, UserId};

use self::federation_commands::{
	disable_room, enable_room, fetch_support_well_known, incoming_federeation, remote_user_in_rooms,
};
use crate::Result;

pub(crate) mod federation_commands;

#[cfg_attr(test, derive(Debug))]
#[derive(Subcommand)]
pub(crate) enum FederationCommand {
	/// - List all rooms we are currently handling an incoming pdu from
	IncomingFederation,

	/// - Disables incoming federation handling for a room.
	DisableRoom {
		room_id: Box<RoomId>,
	},

	/// - Enables incoming federation handling for a room again.
	EnableRoom {
		room_id: Box<RoomId>,
	},

	/// - Fetch `/.well-known/matrix/support` from the specified server
	///
	/// Despite the name, this is not a federation endpoint and does not go
	/// through the federation / server resolution process as per-spec this is
	/// supposed to be served at the server_name.
	///
	/// Respecting homeservers put this file here for listing administration,
	/// moderation, and security inquiries. This command provides a way to
	/// easily fetch that information.
	FetchSupportWellKnown {
		server_name: Box<ServerName>,
	},

	/// - Lists all the rooms we share/track with the specified *remote* user
	RemoteUserInRooms {
		user_id: Box<UserId>,
	},
}

pub(crate) async fn process(command: FederationCommand, body: Vec<&str>) -> Result<RoomMessageEventContent> {
	Ok(match command {
		FederationCommand::DisableRoom {
			room_id,
		} => disable_room(body, room_id).await?,
		FederationCommand::EnableRoom {
			room_id,
		} => enable_room(body, room_id).await?,
		FederationCommand::IncomingFederation => incoming_federeation(body).await?,
		FederationCommand::FetchSupportWellKnown {
			server_name,
		} => fetch_support_well_known(body, server_name).await?,
		FederationCommand::RemoteUserInRooms {
			user_id,
		} => remote_user_in_rooms(body, user_id).await?,
	})
}
