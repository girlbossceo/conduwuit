use axum::extract::State;
use conduit::utils::{IterStream, ReadyExt};
use futures::StreamExt;
use ruma::{
	api::{client::error::ErrorKind, federation::membership::prepare_join_event},
	events::{
		room::{
			join_rules::{AllowRule, JoinRule, RoomJoinRulesEventContent},
			member::{MembershipState, RoomMemberEventContent},
		},
		StateEventType,
	},
	CanonicalJsonObject, RoomId, RoomVersionId, UserId,
};
use serde_json::value::to_raw_value;
use tracing::warn;

use crate::{
	service::{pdu::PduBuilder, Services},
	Error, Result, Ruma,
};

/// # `GET /_matrix/federation/v1/make_join/{roomId}/{userId}`
///
/// Creates a join template.
pub(crate) async fn create_join_event_template_route(
	State(services): State<crate::State>, body: Ruma<prepare_join_event::v1::Request>,
) -> Result<prepare_join_event::v1::Response> {
	if !services.rooms.metadata.exists(&body.room_id).await {
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
	services
		.rooms
		.event_handler
		.acl_check(origin, &body.room_id)
		.await?;

	if services
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
		if services
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

	let room_version_id = services.rooms.state.get_room_version(&body.room_id).await?;

	let state_lock = services.rooms.state.mutex.lock(&body.room_id).await;

	let join_authorized_via_users_server = if (services
		.rooms
		.state_cache
		.is_left(&body.user_id, &body.room_id)
		.await)
		&& user_can_perform_restricted_join(&services, &body.user_id, &body.room_id, &room_version_id).await?
	{
		let auth_user = services
			.rooms
			.state_cache
			.room_members(&body.room_id)
			.ready_filter(|user| user.server_name() == services.globals.server_name())
			.filter(|user| {
				services
					.rooms
					.state_accessor
					.user_can_invite(&body.room_id, user, &body.user_id, &state_lock)
			})
			.boxed()
			.next()
			.await
			.map(ToOwned::to_owned);

		if auth_user.is_some() {
			auth_user
		} else {
			return Err(Error::BadRequest(
				ErrorKind::UnableToGrantJoin,
				"No user on this server is able to assist in joining.",
			));
		}
	} else {
		None
	};

	let room_version_id = services.rooms.state.get_room_version(&body.room_id).await?;
	if !body.ver.contains(&room_version_id) {
		return Err(Error::BadRequest(
			ErrorKind::IncompatibleRoomVersion {
				room_version: room_version_id,
			},
			"Room version not supported.",
		));
	}

	let (_pdu, mut pdu_json) = services
		.rooms
		.timeline
		.create_hash_and_sign_event(
			PduBuilder::state(
				body.user_id.to_string(),
				&RoomMemberEventContent {
					join_authorized_via_users_server,
					..RoomMemberEventContent::new(MembershipState::Join)
				},
			),
			&body.user_id,
			&body.room_id,
			&state_lock,
		)
		.await?;

	drop(state_lock);

	// room v3 and above removed the "event_id" field from remote PDU format
	maybe_strip_event_id(&mut pdu_json, &room_version_id)?;

	Ok(prepare_join_event::v1::Response {
		room_version: Some(room_version_id),
		event: to_raw_value(&pdu_json).expect("CanonicalJson can be serialized to JSON"),
	})
}

/// Checks whether the given user can join the given room via a restricted join.
/// This doesn't check the current user's membership. This should be done
/// externally, either by using the state cache or attempting to authorize the
/// event.
pub(crate) async fn user_can_perform_restricted_join(
	services: &Services, user_id: &UserId, room_id: &RoomId, room_version_id: &RoomVersionId,
) -> Result<bool> {
	use RoomVersionId::*;

	let join_rules_event = services
		.rooms
		.state_accessor
		.room_state_get(room_id, &StateEventType::RoomJoinRules, "")
		.await;

	let Ok(Ok(join_rules_event_content)) = join_rules_event.as_ref().map(|join_rules_event| {
		serde_json::from_str::<RoomJoinRulesEventContent>(join_rules_event.content.get()).map_err(|e| {
			warn!("Invalid join rules event in database: {e}");
			Error::bad_database("Invalid join rules event in database")
		})
	}) else {
		return Ok(false);
	};

	if matches!(room_version_id, V1 | V2 | V3 | V4 | V5 | V6 | V7) {
		return Ok(false);
	}

	let (JoinRule::Restricted(r) | JoinRule::KnockRestricted(r)) = join_rules_event_content.join_rule else {
		return Ok(false);
	};

	if r.allow
		.iter()
		.filter_map(|rule| {
			if let AllowRule::RoomMembership(membership) = rule {
				Some(membership)
			} else {
				None
			}
		})
		.stream()
		.any(|m| services.rooms.state_cache.is_joined(user_id, &m.room_id))
		.await
	{
		Ok(true)
	} else {
		Err(Error::BadRequest(
			ErrorKind::UnableToAuthorizeJoin,
			"User is not known to be in any required room.",
		))
	}
}

pub(crate) fn maybe_strip_event_id(pdu_json: &mut CanonicalJsonObject, room_version_id: &RoomVersionId) -> Result<()> {
	use RoomVersionId::*;

	match room_version_id {
		V1 | V2 => {},
		_ => {
			pdu_json.remove("event_id");
		},
	};

	Ok(())
}
