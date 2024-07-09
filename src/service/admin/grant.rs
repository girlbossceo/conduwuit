use std::collections::BTreeMap;

use conduit::Result;
use ruma::{
	events::{
		room::{
			member::{MembershipState, RoomMemberEventContent},
			message::RoomMessageEventContent,
			power_levels::RoomPowerLevelsEventContent,
		},
		TimelineEventType,
	},
	UserId,
};
use serde_json::value::to_raw_value;

use super::Service;
use crate::{pdu::PduBuilder, services};

/// Invite the user to the conduit admin room.
///
/// In conduit, this is equivalent to granting admin privileges.
pub async fn make_user_admin(user_id: &UserId, displayname: String) -> Result<()> {
	if let Some(room_id) = Service::get_admin_room()? {
		let state_lock = services().rooms.state.mutex.lock(&room_id).await;

		// Use the server user to grant the new admin's power level
		let server_user = &services().globals.server_user;

		// Invite and join the real user
		services()
			.rooms
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
				},
				server_user,
				&room_id,
				&state_lock,
			)
			.await?;
		services()
			.rooms
			.timeline
			.build_and_append_pdu(
				PduBuilder {
					event_type: TimelineEventType::RoomMember,
					content: to_raw_value(&RoomMemberEventContent {
						membership: MembershipState::Join,
						displayname: Some(displayname),
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
				},
				user_id,
				&room_id,
				&state_lock,
			)
			.await?;

		// Set power level
		let mut users = BTreeMap::new();
		users.insert(server_user.clone(), 100.into());
		users.insert(user_id.to_owned(), 100.into());

		services()
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
				},
				server_user,
				&room_id,
				&state_lock,
			)
			.await?;

		// Send welcome message
		services().rooms.timeline.build_and_append_pdu(
  			PduBuilder {
                event_type: TimelineEventType::RoomMessage,
                content: to_raw_value(&RoomMessageEventContent::text_html(
                        format!("## Thank you for trying out conduwuit!\n\nconduwuit is a fork of upstream Conduit which is in Beta. This means you can join and participate in most Matrix rooms, but not all features are supported and you might run into bugs from time to time.\n\nHelpful links:\n> Git and Documentation: https://github.com/girlbossceo/conduwuit\n> Report issues: https://github.com/girlbossceo/conduwuit/issues\n\nFor a list of available commands, send the following message in this room: `@conduit:{}: --help`\n\nHere are some rooms you can join (by typing the command):\n\nconduwuit room (Ask questions and get notified on updates):\n`/join #conduwuit:puppygock.gay`", services().globals.server_name()),
                        format!("<h2>Thank you for trying out conduwuit!</h2>\n<p>conduwuit is a fork of upstream Conduit which is in Beta. This means you can join and participate in most Matrix rooms, but not all features are supported and you might run into bugs from time to time.</p>\n<p>Helpful links:</p>\n<blockquote>\n<p>Git and Documentation: https://github.com/girlbossceo/conduwuit<br>Report issues: https://github.com/girlbossceo/conduwuit/issues</p>\n</blockquote>\n<p>For a list of available commands, send the following message in this room: <code>@conduit:{}: --help</code></p>\n<p>Here are some rooms you can join (by typing the command):</p>\n<p>conduwuit room (Ask questions and get notified on updates):<br><code>/join #conduwuit:puppygock.gay</code></p>\n", services().globals.server_name()),
                ))
                .expect("event is valid, we just created it"),
                unsigned: None,
                state_key: None,
                redacts: None,
            },
            server_user,
            &room_id,
            &state_lock,
        ).await?;
	}

	Ok(())
}
