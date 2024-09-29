use conduit::Result;
use futures::StreamExt;
use ruma::{events::room::message::RoomMessageEventContent, OwnedRoomId};

use crate::{admin_command, get_room_info, PAGE_SIZE};

#[admin_command]
pub(super) async fn list_rooms(
	&self, page: Option<usize>, exclude_disabled: bool, exclude_banned: bool, no_details: bool,
) -> Result<RoomMessageEventContent> {
	// TODO: i know there's a way to do this with clap, but i can't seem to find it
	let page = page.unwrap_or(1);
	let mut rooms = self
		.services
		.rooms
		.metadata
		.iter_ids()
		.filter_map(|room_id| async move {
			(!exclude_disabled || !self.services.rooms.metadata.is_disabled(room_id).await).then_some(room_id)
		})
		.filter_map(|room_id| async move {
			(!exclude_banned || !self.services.rooms.metadata.is_banned(room_id).await).then_some(room_id)
		})
		.then(|room_id| get_room_info(self.services, room_id))
		.collect::<Vec<_>>()
		.await;

	rooms.sort_by_key(|r| r.1);
	rooms.reverse();

	let rooms = rooms
		.into_iter()
		.skip(page.saturating_sub(1).saturating_mul(PAGE_SIZE))
		.take(PAGE_SIZE)
		.collect::<Vec<_>>();

	if rooms.is_empty() {
		return Ok(RoomMessageEventContent::text_plain("No more rooms."));
	};

	let output_plain = format!(
		"Rooms ({}):\n```\n{}\n```",
		rooms.len(),
		rooms
			.iter()
			.map(|(id, members, name)| if no_details {
				format!("{id}")
			} else {
				format!("{id}\tMembers: {members}\tName: {name}")
			})
			.collect::<Vec<_>>()
			.join("\n")
	);

	Ok(RoomMessageEventContent::notice_markdown(output_plain))
}

#[admin_command]
pub(super) async fn exists(&self, room_id: OwnedRoomId) -> Result<RoomMessageEventContent> {
	let result = self.services.rooms.metadata.exists(&room_id).await;

	Ok(RoomMessageEventContent::notice_markdown(format!("{result}")))
}
