use clap::Subcommand;
use conduwuit::Result;
use ruma::{OwnedEventId, OwnedRoomOrAliasId, events::room::message::RoomMessageEventContent};

use crate::{admin_command, admin_command_dispatch};

#[admin_command_dispatch]
#[derive(Debug, Subcommand)]
/// Query tables from database
pub(crate) enum ShortCommand {
	ShortEventId {
		event_id: OwnedEventId,
	},

	ShortRoomId {
		room_id: OwnedRoomOrAliasId,
	},
}

#[admin_command]
pub(super) async fn short_event_id(
	&self,
	event_id: OwnedEventId,
) -> Result<RoomMessageEventContent> {
	let shortid = self
		.services
		.rooms
		.short
		.get_shorteventid(&event_id)
		.await?;

	Ok(RoomMessageEventContent::notice_markdown(format!("{shortid:#?}")))
}

#[admin_command]
pub(super) async fn short_room_id(
	&self,
	room_id: OwnedRoomOrAliasId,
) -> Result<RoomMessageEventContent> {
	let room_id = self.services.rooms.alias.resolve(&room_id).await?;

	let shortid = self.services.rooms.short.get_shortroomid(&room_id).await?;

	Ok(RoomMessageEventContent::notice_markdown(format!("{shortid:#?}")))
}
