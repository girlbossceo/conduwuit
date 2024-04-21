use clap::Subcommand;
use ruma::{events::room::message::RoomMessageEventContent, EventId, RoomId, ServerName};

use self::debug_commands::{
	change_log_level, force_device_list_updates, get_auth_chain, get_pdu, get_remote_pdu, get_room_state, parse_pdu,
	ping, sign_json, verify_json,
};
use crate::Result;

pub(crate) mod debug_commands;

#[cfg_attr(test, derive(Debug))]
#[derive(Subcommand)]
pub(crate) enum DebugCommand {
	/// - Get the auth_chain of a PDU
	GetAuthChain {
		/// An event ID (the $ character followed by the base64 reference hash)
		event_id: Box<EventId>,
	},

	/// - Parse and print a PDU from a JSON
	///
	/// The PDU event is only checked for validity and is not added to the
	/// database.
	///
	/// This command needs a JSON blob provided in a Markdown code block below
	/// the command.
	ParsePdu,

	/// - Retrieve and print a PDU by ID from the conduwuit database
	GetPdu {
		/// An event ID (a $ followed by the base64 reference hash)
		event_id: Box<EventId>,
	},

	/// - Attempts to retrieve a PDU from a remote server. Inserts it into our
	/// database/timeline if found and we do not have this PDU already
	/// (following normal event auth rules, handles it as an incoming PDU).
	GetRemotePdu {
		/// An event ID (a $ followed by the base64 reference hash)
		event_id: Box<EventId>,

		/// Argument for us to attempt to fetch the event from the
		/// specified remote server.
		server: Box<ServerName>,
	},

	/// - Gets all the room state events for the specified room.
	///
	/// This is functionally equivalent to `GET
	/// /_matrix/client/v3/rooms/{roomid}/state`, except the admin command does
	/// *not* check if the sender user is allowed to see state events. This is
	/// done because it's implied that server admins here have database access
	/// and can see/get room info themselves anyways if they were malicious
	/// admins.
	///
	/// Of course the check is still done on the actual client API.
	GetRoomState {
		/// Room ID
		room_id: Box<RoomId>,
	},

	/// - Sends a federation request to the remote server's
	///   `/_matrix/federation/v1/version` endpoint and measures the latency it
	///   took for the server to respond
	Ping {
		server: Box<ServerName>,
	},

	/// - Forces device lists for all local and remote users to be updated (as
	///   having new keys available)
	ForceDeviceListUpdates,

	/// - Change tracing log level/filter on the fly
	///
	/// This accepts the same format as the `log` config option.
	ChangeLogLevel {
		/// Log level/filter
		filter: Option<String>,

		/// Resets the log level/filter to the one in your config
		#[arg(short, long)]
		reset: bool,
	},

	/// - Verify json signatures
	///
	/// This command needs a JSON blob provided in a Markdown code block below
	/// the command.
	SignJson,

	/// - Verify json signatures
	///
	/// This command needs a JSON blob provided in a Markdown code block below
	/// the command.
	VerifyJson,
}

pub(crate) async fn process(command: DebugCommand, body: Vec<&str>) -> Result<RoomMessageEventContent> {
	Ok(match command {
		DebugCommand::GetAuthChain {
			event_id,
		} => get_auth_chain(body, event_id).await?,
		DebugCommand::ParsePdu => parse_pdu(body).await?,
		DebugCommand::GetPdu {
			event_id,
		} => get_pdu(body, event_id).await?,
		DebugCommand::GetRemotePdu {
			event_id,
			server,
		} => get_remote_pdu(body, event_id, server).await?,
		DebugCommand::GetRoomState {
			room_id,
		} => get_room_state(body, room_id).await?,
		DebugCommand::Ping {
			server,
		} => ping(body, server).await?,
		DebugCommand::ForceDeviceListUpdates => force_device_list_updates(body).await?,
		DebugCommand::ChangeLogLevel {
			filter,
			reset,
		} => change_log_level(body, filter, reset).await?,
		DebugCommand::SignJson => sign_json(body).await?,
		DebugCommand::VerifyJson => verify_json(body).await?,
	})
}
