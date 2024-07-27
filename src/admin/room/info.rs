use clap::Subcommand;
use conduit::Result;
use ruma::{events::room::message::RoomMessageEventContent, RoomId};

use crate::{admin_command, admin_command_dispatch};

#[admin_command_dispatch]
#[derive(Debug, Subcommand)]
pub(crate) enum RoomInfoCommand {
	/// - List joined members in a room
	ListJoinedMembers {
		room_id: Box<RoomId>,
	},

	/// - Displays room topic
	///
	/// Room topics can be huge, so this is in its
	/// own separate command
	ViewRoomTopic {
		room_id: Box<RoomId>,
	},
}

#[admin_command]
async fn list_joined_members(&self, room_id: Box<RoomId>) -> Result<RoomMessageEventContent> {
	let room_name = self
		.services
		.rooms
		.state_accessor
		.get_name(&room_id)
		.ok()
		.flatten()
		.unwrap_or_else(|| room_id.to_string());

	let members = self
		.services
		.rooms
		.state_cache
		.room_members(&room_id)
		.filter_map(Result::ok);

	let member_info = members
		.into_iter()
		.map(|user_id| {
			(
				user_id.clone(),
				self.services
					.users
					.displayname(&user_id)
					.unwrap_or(None)
					.unwrap_or_else(|| user_id.to_string()),
			)
		})
		.collect::<Vec<_>>();

	let output_plain = format!(
		"{} Members in Room \"{}\":\n```\n{}\n```",
		member_info.len(),
		room_name,
		member_info
			.iter()
			.map(|(mxid, displayname)| format!("{mxid} | {displayname}"))
			.collect::<Vec<_>>()
			.join("\n")
	);

	Ok(RoomMessageEventContent::notice_markdown(output_plain))
}

#[admin_command]
async fn view_room_topic(&self, room_id: Box<RoomId>) -> Result<RoomMessageEventContent> {
	let Some(room_topic) = self
		.services
		.rooms
		.state_accessor
		.get_room_topic(&room_id)?
	else {
		return Ok(RoomMessageEventContent::text_plain("Room does not have a room topic set."));
	};

	Ok(RoomMessageEventContent::notice_markdown(format!(
		"Room topic:\n```\n{room_topic}\n```"
	)))
}
