use std::collections::BTreeMap;

use conduit::{error, implement, Result};
use ruma::{
	events::{
		room::{
			member::{MembershipState, RoomMemberEventContent},
			message::RoomMessageEventContent,
			power_levels::RoomPowerLevelsEventContent,
		},
		tag::{TagEvent, TagEventContent, TagInfo},
		RoomAccountDataEventType, TimelineEventType,
	},
	RoomId, UserId,
};
use serde_json::value::to_raw_value;

use crate::pdu::PduBuilder;

/// Invite the user to the conduit admin room.
///
/// In conduit, this is equivalent to granting admin privileges.
#[implement(super::Service)]
pub async fn make_user_admin(&self, user_id: &UserId) -> Result<()> {
	let Ok(room_id) = self.get_admin_room().await else {
		return Ok(());
	};

	let state_lock = self.services.state.mutex.lock(&room_id).await;

	// Use the server user to grant the new admin's power level
	let server_user = &self.services.globals.server_user;

	// Invite and join the real user
	self.services
		.timeline
		.build_and_append_pdu(
			PduBuilder {
				event_type: TimelineEventType::RoomMember,
				content: to_raw_value(&RoomMemberEventContent {
					membership: MembershipState::Invite,
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
				state_key: Some(user_id.to_string()),
				redacts: None,
				timestamp: None,
			},
			server_user,
			&room_id,
			&state_lock,
		)
		.await?;
	self.services
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
				state_key: Some(user_id.to_string()),
				redacts: None,
				timestamp: None,
			},
			user_id,
			&room_id,
			&state_lock,
		)
		.await?;

	// Set power level
	let users = BTreeMap::from_iter([(server_user.clone(), 100.into()), (user_id.to_owned(), 100.into())]);

	self.services
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

	// Set room tag
	let room_tag = &self.services.server.config.admin_room_tag;
	if !room_tag.is_empty() {
		if let Err(e) = self.set_room_tag(&room_id, user_id, room_tag).await {
			error!(?room_id, ?user_id, ?room_tag, ?e, "Failed to set tag for admin grant");
		}
	}

	// Send welcome message
	self.services.timeline.build_and_append_pdu(
  			PduBuilder {
                event_type: TimelineEventType::RoomMessage,
                content: to_raw_value(&RoomMessageEventContent::text_markdown(
                        String::from("## Thank you for trying out conduwuit!\n\nconduwuit is a fork of upstream Conduit which is in Beta. This means you can join and participate in most Matrix rooms, but not all features are supported and you might run into bugs from time to time.\n\nHelpful links:\n> Git and Documentation: https://github.com/girlbossceo/conduwuit\n> Report issues: https://github.com/girlbossceo/conduwuit/issues\n\nFor a list of available commands, send the following message in this room: `!admin --help`\n\nHere are some rooms you can join (by typing the command):\n\nconduwuit room (Ask questions and get notified on updates):\n`/join #conduwuit:puppygock.gay`"),
                ))
                .expect("event is valid, we just created it"),
                unsigned: None,
                state_key: None,
                redacts: None,
                timestamp: None,
            },
            server_user,
            &room_id,
            &state_lock,
        ).await?;

	Ok(())
}

#[implement(super::Service)]
async fn set_room_tag(&self, room_id: &RoomId, user_id: &UserId, tag: &str) -> Result<()> {
	let mut event = self
		.services
		.account_data
		.get_room(room_id, user_id, RoomAccountDataEventType::Tag)
		.await
		.unwrap_or_else(|_| TagEvent {
			content: TagEventContent {
				tags: BTreeMap::new(),
			},
		});

	event
		.content
		.tags
		.insert(tag.to_owned().into(), TagInfo::new());

	self.services
		.account_data
		.update(
			Some(room_id),
			user_id,
			RoomAccountDataEventType::Tag,
			&serde_json::to_value(event)?,
		)
		.await?;

	Ok(())
}
