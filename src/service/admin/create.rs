use std::collections::BTreeMap;

use conduit::{pdu::PduBuilder, Result};
use ruma::{
	events::{
		room::{
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
		TimelineEventType,
	},
	RoomId, RoomVersionId,
};
use serde_json::value::to_raw_value;

use crate::Services;

/// Create the admin room.
///
/// Users in this room are considered admins by conduit, and the room can be
/// used to issue admin commands by talking to the server user inside it.
pub async fn create_admin_room(services: &Services) -> Result<()> {
	let room_id = RoomId::new(services.globals.server_name());

	let _short_id = services.rooms.short.get_or_create_shortroomid(&room_id);

	let state_lock = services.rooms.state.mutex.lock(&room_id).await;

	// Create a user for the server
	let server_user = &services.globals.server_user;
	services.users.create(server_user, None)?;

	let room_version = services.globals.default_room_version();

	let mut content = {
		use RoomVersionId::*;
		match room_version {
			V1 | V2 | V3 | V4 | V5 | V6 | V7 | V8 | V9 | V10 => RoomCreateEventContent::new_v1(server_user.clone()),
			_ => RoomCreateEventContent::new_v11(),
		}
	};

	content.federate = true;
	content.predecessor = None;
	content.room_version = room_version;

	// 1. The room create event
	services
		.rooms
		.timeline
		.build_and_append_pdu(
			PduBuilder {
				event_type: TimelineEventType::RoomCreate,
				content: to_raw_value(&content).expect("event is valid, we just created it"),
				unsigned: None,
				state_key: Some(String::new()),
				redacts: None,
				timestamp: None,
			},
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
			PduBuilder {
				event_type: TimelineEventType::RoomMember,
				content: to_raw_value(&RoomMemberEventContent {
					membership: MembershipState::Join,
					displayname: None,
					avatar_url: None,
					is_direct: None,
					third_party_invite: None,
					blurhash: None,
					reason: None,
					join_authorized_via_users_server: None,
				})
				.expect("event is valid, we just created it"),
				unsigned: None,
				state_key: Some(server_user.to_string()),
				redacts: None,
				timestamp: None,
			},
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
			PduBuilder {
				event_type: TimelineEventType::RoomPowerLevels,
				content: to_raw_value(&RoomPowerLevelsEventContent {
					users,
					..Default::default()
				})
				.expect("event is valid, we just created it"),
				unsigned: None,
				state_key: Some(String::new()),
				redacts: None,
				timestamp: None,
			},
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
			PduBuilder {
				event_type: TimelineEventType::RoomJoinRules,
				content: to_raw_value(&RoomJoinRulesEventContent::new(JoinRule::Invite))
					.expect("event is valid, we just created it"),
				unsigned: None,
				state_key: Some(String::new()),
				redacts: None,
				timestamp: None,
			},
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
			PduBuilder {
				event_type: TimelineEventType::RoomHistoryVisibility,
				content: to_raw_value(&RoomHistoryVisibilityEventContent::new(HistoryVisibility::Shared))
					.expect("event is valid, we just created it"),
				unsigned: None,
				state_key: Some(String::new()),
				redacts: None,
				timestamp: None,
			},
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
			PduBuilder {
				event_type: TimelineEventType::RoomGuestAccess,
				content: to_raw_value(&RoomGuestAccessEventContent::new(GuestAccess::Forbidden))
					.expect("event is valid, we just created it"),
				unsigned: None,
				state_key: Some(String::new()),
				redacts: None,
				timestamp: None,
			},
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
			PduBuilder {
				event_type: TimelineEventType::RoomName,
				content: to_raw_value(&RoomNameEventContent::new(room_name))
					.expect("event is valid, we just created it"),
				unsigned: None,
				state_key: Some(String::new()),
				redacts: None,
				timestamp: None,
			},
			server_user,
			&room_id,
			&state_lock,
		)
		.await?;

	services
		.rooms
		.timeline
		.build_and_append_pdu(
			PduBuilder {
				event_type: TimelineEventType::RoomTopic,
				content: to_raw_value(&RoomTopicEventContent {
					topic: format!("Manage {}", services.globals.server_name()),
				})
				.expect("event is valid, we just created it"),
				unsigned: None,
				state_key: Some(String::new()),
				redacts: None,
				timestamp: None,
			},
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
			PduBuilder {
				event_type: TimelineEventType::RoomCanonicalAlias,
				content: to_raw_value(&RoomCanonicalAliasEventContent {
					alias: Some(alias.clone()),
					alt_aliases: Vec::new(),
				})
				.expect("event is valid, we just created it"),
				unsigned: None,
				state_key: Some(String::new()),
				redacts: None,
				timestamp: None,
			},
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
			PduBuilder {
				event_type: TimelineEventType::RoomPreviewUrls,
				content: to_raw_value(&RoomPreviewUrlsEventContent {
					disabled: true,
				})
				.expect("event is valid we just created it"),
				unsigned: None,
				state_key: Some(String::new()),
				redacts: None,
				timestamp: None,
			},
			server_user,
			&room_id,
			&state_lock,
		)
		.await?;

	Ok(())
}
