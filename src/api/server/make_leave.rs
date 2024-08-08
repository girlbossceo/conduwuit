use axum::extract::State;
use conduit::{Error, Result};
use ruma::{
	api::{client::error::ErrorKind, federation::membership::prepare_leave_event},
	events::{
		room::member::{MembershipState, RoomMemberEventContent},
		TimelineEventType,
	},
};
use serde_json::value::to_raw_value;

use super::make_join::maybe_strip_event_id;
use crate::{service::pdu::PduBuilder, Ruma};

/// # `PUT /_matrix/federation/v1/make_leave/{roomId}/{eventId}`
///
/// Creates a leave template.
pub(crate) async fn create_leave_event_template_route(
	State(services): State<crate::State>, body: Ruma<prepare_leave_event::v1::Request>,
) -> Result<prepare_leave_event::v1::Response> {
	if !services.rooms.metadata.exists(&body.room_id).await {
		return Err(Error::BadRequest(ErrorKind::NotFound, "Room is unknown to this server."));
	}

	let origin = body.origin.as_ref().expect("server is authenticated");
	if body.user_id.server_name() != origin {
		return Err(Error::BadRequest(
			ErrorKind::InvalidParam,
			"Not allowed to leave on behalf of another server/user",
		));
	}

	// ACL check origin
	services
		.rooms
		.event_handler
		.acl_check(origin, &body.room_id)
		.await?;

	let room_version_id = services.rooms.state.get_room_version(&body.room_id).await?;
	let state_lock = services.rooms.state.mutex.lock(&body.room_id).await;
	let content = to_raw_value(&RoomMemberEventContent {
		avatar_url: None,
		blurhash: None,
		displayname: None,
		is_direct: None,
		membership: MembershipState::Leave,
		third_party_invite: None,
		reason: None,
		join_authorized_via_users_server: None,
	})
	.expect("member event is valid value");

	let (_pdu, mut pdu_json) = services
		.rooms
		.timeline
		.create_hash_and_sign_event(
			PduBuilder {
				event_type: TimelineEventType::RoomMember,
				content,
				unsigned: None,
				state_key: Some(body.user_id.to_string()),
				redacts: None,
				timestamp: None,
			},
			&body.user_id,
			&body.room_id,
			&state_lock,
		)
		.await?;

	drop(state_lock);

	// room v3 and above removed the "event_id" field from remote PDU format
	maybe_strip_event_id(&mut pdu_json, &room_version_id)?;

	Ok(prepare_leave_event::v1::Response {
		room_version: Some(room_version_id),
		event: to_raw_value(&pdu_json).expect("CanonicalJson can be serialized to JSON"),
	})
}
