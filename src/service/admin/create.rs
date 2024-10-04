use std::collections::BTreeMap;

use conduit::{pdu::PduBuilder, Result};
use ruma::{
	events::room::{
		canonical_alias::RoomCanonicalAliasEventContent,
		create::RoomCreateEventContent,
		guest_access::{GuestAccess, RoomGuestAccessEventContent},
		history_visibility::{HistoryVisibility, RoomHistoryVisibilityEventContent},
		join_rules::{JoinRule, RoomJoinRulesEventContent},
		member::{MembershipState, RoomMemberEventContent},
		name::RoomNameEventContent,
		power_levels::RoomPowerLevelsEventContent,
		preview_url::RoomPreviewUrlsEventContent,
		topic::RoomTopicEventContent,
	},
	RoomId, RoomVersionId,
};

use crate::Services;

/// Create the admin room.
///
/// Users in this room are considered admins by conduit, and the room can be
/// used to issue admin commands by talking to the server user inside it.
pub async fn create_admin_room(services: &Services) -> Result<()> {
	let room_id = RoomId::new(services.globals.server_name());

	let _short_id = services
		.rooms
		.short
		.get_or_create_shortroomid(&room_id)
		.await;

	let state_lock = services.rooms.state.mutex.lock(&room_id).await;

	// Create a user for the server
	let server_user = &services.globals.server_user;
	services.users.create(server_user, None)?;

	let room_version = services.globals.default_room_version();

	let create_content = {
		use RoomVersionId::*;
		match room_version {
			V1 | V2 | V3 | V4 | V5 | V6 | V7 | V8 | V9 | V10 => RoomCreateEventContent::new_v1(server_user.clone()),
			_ => RoomCreateEventContent::new_v11(),
		}
	};

	// 1. The room create event
	services
		.rooms
		.timeline
		.build_and_append_pdu(
			PduBuilder::state(
				String::new(),
				&RoomCreateEventContent {
					federate: true,
					predecessor: None,
					room_version,
					..create_content
				},
			),
			server_user,
			&room_id,
			&state_lock,
		)
		.await?;

	// 2. Make conduit bot join
	services
		.rooms
		.timeline
		.build_and_append_pdu(
			PduBuilder::state(server_user.to_string(), &RoomMemberEventContent::new(MembershipState::Join)),
			server_user,
			&room_id,
			&state_lock,
		)
		.await?;

	// 3. Power levels
	let users = BTreeMap::from_iter([(server_user.clone(), 100.into())]);

	services
		.rooms
		.timeline
		.build_and_append_pdu(
			PduBuilder::state(
				String::new(),
				&RoomPowerLevelsEventContent {
					users,
					..Default::default()
				},
			),
			server_user,
			&room_id,
			&state_lock,
		)
		.await?;

	// 4.1 Join Rules
	services
		.rooms
		.timeline
		.build_and_append_pdu(
			PduBuilder::state(String::new(), &RoomJoinRulesEventContent::new(JoinRule::Invite)),
			server_user,
			&room_id,
			&state_lock,
		)
		.await?;

	// 4.2 History Visibility
	services
		.rooms
		.timeline
		.build_and_append_pdu(
			PduBuilder::state(
				String::new(),
				&RoomHistoryVisibilityEventContent::new(HistoryVisibility::Shared),
			),
			server_user,
			&room_id,
			&state_lock,
		)
		.await?;

	// 4.3 Guest Access
	services
		.rooms
		.timeline
		.build_and_append_pdu(
			PduBuilder::state(String::new(), &RoomGuestAccessEventContent::new(GuestAccess::Forbidden)),
			server_user,
			&room_id,
			&state_lock,
		)
		.await?;

	// 5. Events implied by name and topic
	let room_name = format!("{} Admin Room", services.globals.server_name());
	services
		.rooms
		.timeline
		.build_and_append_pdu(
			PduBuilder::state(String::new(), &RoomNameEventContent::new(room_name)),
			server_user,
			&room_id,
			&state_lock,
		)
		.await?;

	services
		.rooms
		.timeline
		.build_and_append_pdu(
			PduBuilder::state(
				String::new(),
				&RoomTopicEventContent {
					topic: format!("Manage {}", services.globals.server_name()),
				},
			),
			server_user,
			&room_id,
			&state_lock,
		)
		.await?;

	// 6. Room alias
	let alias = &services.globals.admin_alias;

	services
		.rooms
		.timeline
		.build_and_append_pdu(
			PduBuilder::state(
				String::new(),
				&RoomCanonicalAliasEventContent {
					alias: Some(alias.clone()),
					alt_aliases: Vec::new(),
				},
			),
			server_user,
			&room_id,
			&state_lock,
		)
		.await?;

	services
		.rooms
		.alias
		.set_alias(alias, &room_id, server_user)?;

	// 7. (ad-hoc) Disable room previews for everyone by default
	services
		.rooms
		.timeline
		.build_and_append_pdu(
			PduBuilder::state(
				String::new(),
				&RoomPreviewUrlsEventContent {
					disabled: true,
				},
			),
			server_user,
			&room_id,
			&state_lock,
		)
		.await?;

	Ok(())
}
