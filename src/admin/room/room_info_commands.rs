use ruma::{events::room::message::RoomMessageEventContent, RoomId};
use service::services;

use super::RoomInfoCommand;
use crate::Result;

pub(super) async fn process(command: RoomInfoCommand, body: Vec<&str>) -> Result<RoomMessageEventContent> {
	match command {
		RoomInfoCommand::ListJoinedMembers {
			room_id,
		} => list_joined_members(body, room_id).await,
		RoomInfoCommand::ViewRoomTopic {
			room_id,
		} => view_room_topic(body, room_id).await,
	}
}

async fn list_joined_members(_body: Vec<&str>, room_id: Box<RoomId>) -> Result<RoomMessageEventContent> {
	let room_name = services()
		.rooms
		.state_accessor
		.get_name(&room_id)
		.ok()
		.flatten()
		.unwrap_or_else(|| room_id.to_string());

	let members = services()
		.rooms
		.state_cache
		.room_members(&room_id)
		.filter_map(Result::ok);

	let member_info = members
		.into_iter()
		.map(|user_id| {
			(
				user_id.clone(),
				services()
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

async fn view_room_topic(_body: Vec<&str>, room_id: Box<RoomId>) -> Result<RoomMessageEventContent> {
	let Some(room_topic) = services().rooms.state_accessor.get_room_topic(&room_id)? else {
		return Ok(RoomMessageEventContent::text_plain("Room does not have a room topic set."));
	};

	Ok(RoomMessageEventContent::notice_markdown(format!(
		"Room topic:\n```\n{room_topic}\n```"
	)))
}
