use std::fmt::Write;

use ruma::{events::room::message::RoomMessageEventContent, RoomId};
use service::services;

use super::RoomInfoCommand;
use crate::{escape_html, Result};

pub(crate) async fn process(command: RoomInfoCommand, body: Vec<&str>) -> Result<RoomMessageEventContent> {
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

	let output_html = format!(
		"<table><caption>{} Members in Room \"{}\" </caption>\n<tr><th>MXID</th>\t<th>Display \
		 Name</th></tr>\n{}</table>",
		member_info.len(),
		room_name,
		member_info
			.iter()
			.fold(String::new(), |mut output, (mxid, displayname)| {
				writeln!(
					output,
					"<tr><td>{}</td>\t<td>{}</td></tr>",
					mxid,
					escape_html(displayname.as_ref())
				)
				.expect("should be able to write to string buffer");
				output
			})
	);

	Ok(RoomMessageEventContent::text_html(output_plain, output_html))
}

async fn view_room_topic(_body: Vec<&str>, room_id: Box<RoomId>) -> Result<RoomMessageEventContent> {
	let Some(room_topic) = services().rooms.state_accessor.get_room_topic(&room_id)? else {
		return Ok(RoomMessageEventContent::text_plain("Room does not have a room topic set."));
	};

	let output_html = format!("<p>Room topic:</p>\n<hr>\n{}<hr>", escape_html(&room_topic));

	Ok(RoomMessageEventContent::text_html(
		format!("Room topic:\n\n```{room_topic}\n```"),
		output_html,
	))
}
