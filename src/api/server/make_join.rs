use std::sync::Arc;

use ruma::{
	api::{client::error::ErrorKind, federation::membership::prepare_join_event},
	events::{
		room::{
			join_rules::{AllowRule, JoinRule, RoomJoinRulesEventContent},
			member::{MembershipState, RoomMemberEventContent},
		},
		StateEventType, TimelineEventType,
	},
	RoomVersionId,
};
use serde_json::value::to_raw_value;
use tracing::warn;

use crate::{
	service::{pdu::PduBuilder, user_is_local},
	services, Error, Result, Ruma,
};

/// # `GET /_matrix/federation/v1/make_join/{roomId}/{userId}`
///
/// Creates a join template.
pub(crate) async fn create_join_event_template_route(
	body: Ruma<prepare_join_event::v1::Request>,
) -> Result<prepare_join_event::v1::Response> {
	if !services().rooms.metadata.exists(&body.room_id)? {
		return Err(Error::BadRequest(ErrorKind::NotFound, "Room is unknown to this server."));
	}

	let origin = body.origin.as_ref().expect("server is authenticated");
	if body.user_id.server_name() != origin {
		return Err(Error::BadRequest(
			ErrorKind::InvalidParam,
			"Not allowed to join on behalf of another server/user",
		));
	}

	// ACL check origin server
	services()
		.rooms
		.event_handler
		.acl_check(origin, &body.room_id)?;

	if services()
		.globals
		.config
		.forbidden_remote_server_names
		.contains(origin)
	{
		warn!(
			"Server {origin} for remote user {} tried joining room ID {} which has a server name that is globally \
			 forbidden. Rejecting.",
			&body.user_id, &body.room_id,
		);
		return Err(Error::BadRequest(
			ErrorKind::forbidden(),
			"Server is banned on this homeserver.",
		));
	}

	if let Some(server) = body.room_id.server_name() {
		if services()
			.globals
			.config
			.forbidden_remote_server_names
			.contains(&server.to_owned())
		{
			return Err(Error::BadRequest(
				ErrorKind::forbidden(),
				"Server is banned on this homeserver.",
			));
		}
	}

	let mutex_state = Arc::clone(
		services()
			.globals
			.roomid_mutex_state
			.write()
			.await
			.entry(body.room_id.clone())
			.or_default(),
	);
	let state_lock = mutex_state.lock().await;

	let join_rules_event =
		services()
			.rooms
			.state_accessor
			.room_state_get(&body.room_id, &StateEventType::RoomJoinRules, "")?;

	let join_rules_event_content: Option<RoomJoinRulesEventContent> = join_rules_event
		.as_ref()
		.map(|join_rules_event| {
			serde_json::from_str(join_rules_event.content.get())
				.map_err(|_| Error::bad_database("Invalid join rules event in db."))
		})
		.transpose()?;

	let join_authorized_via_users_server = if let Some(join_rules_event_content) = join_rules_event_content {
		if let JoinRule::Restricted(r) | JoinRule::KnockRestricted(r) = join_rules_event_content.join_rule {
			if r.allow
				.iter()
				.filter_map(|rule| {
					if let AllowRule::RoomMembership(membership) = rule {
						Some(membership)
					} else {
						None
					}
				})
				.any(|m| {
					services()
						.rooms
						.state_cache
						.is_joined(&body.user_id, &m.room_id)
						.unwrap_or(false)
				}) {
				if services()
					.rooms
					.state_cache
					.is_left(&body.user_id, &body.room_id)
					.unwrap_or(true)
				{
					let members: Vec<_> = services()
						.rooms
						.state_cache
						.room_members(&body.room_id)
						.filter_map(Result::ok)
						.filter(|user| user_is_local(user))
						.collect();

					let mut auth_user = None;

					for user in members {
						if services()
							.rooms
							.state_accessor
							.user_can_invite(&body.room_id, &user, &body.user_id, &state_lock)
							.await
							.unwrap_or(false)
						{
							auth_user = Some(user);
							break;
						}
					}
					if auth_user.is_some() {
						auth_user
					} else {
						return Err(Error::BadRequest(
							ErrorKind::UnableToGrantJoin,
							"No user on this server is able to assist in joining.",
						));
					}
				} else {
					// If the user has any state other than leave, either:
					// - the auth_check will deny them (ban, knock - (until/unless MSC4123 is
					//   merged))
					// - they are able to join via other methods (invite)
					// - they are already in the room (join)
					None
				}
			} else {
				return Err(Error::BadRequest(
					ErrorKind::UnableToAuthorizeJoin,
					"User is not known to be in any required room.",
				));
			}
		} else {
			None
		}
	} else {
		None
	};

	let room_version_id = services().rooms.state.get_room_version(&body.room_id)?;
	if !body.ver.contains(&room_version_id) {
		return Err(Error::BadRequest(
			ErrorKind::IncompatibleRoomVersion {
				room_version: room_version_id,
			},
			"Room version not supported.",
		));
	}

	let content = to_raw_value(&RoomMemberEventContent {
		avatar_url: None,
		blurhash: None,
		displayname: None,
		is_direct: None,
		membership: MembershipState::Join,
		third_party_invite: None,
		reason: None,
		join_authorized_via_users_server,
	})
	.expect("member event is valid value");

	let (_pdu, mut pdu_json) = services().rooms.timeline.create_hash_and_sign_event(
		PduBuilder {
			event_type: TimelineEventType::RoomMember,
			content,
			unsigned: None,
			state_key: Some(body.user_id.to_string()),
			redacts: None,
		},
		&body.user_id,
		&body.room_id,
		&state_lock,
	)?;

	drop(state_lock);

	// room v3 and above removed the "event_id" field from remote PDU format
	match room_version_id {
		RoomVersionId::V1 | RoomVersionId::V2 => {},
		RoomVersionId::V3
		| RoomVersionId::V4
		| RoomVersionId::V5
		| RoomVersionId::V6
		| RoomVersionId::V7
		| RoomVersionId::V8
		| RoomVersionId::V9
		| RoomVersionId::V10
		| RoomVersionId::V11 => {
			pdu_json.remove("event_id");
		},
		_ => {
			warn!("Unexpected or unsupported room version {room_version_id}");
			return Err(Error::BadRequest(
				ErrorKind::BadJson,
				"Unexpected or unsupported room version found",
			));
		},
	};

	Ok(prepare_join_event::v1::Response {
		room_version: Some(room_version_id),
		event: to_raw_value(&pdu_json).expect("CanonicalJson can be serialized to JSON"),
	})
}
