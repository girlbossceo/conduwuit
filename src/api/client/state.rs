use std::sync::Arc;

use axum::extract::State;
use conduit::{err, pdu::PduBuilder, utils::BoolExt, Err, Error, Result};
use ruma::{
	api::client::{
		error::ErrorKind,
		state::{get_state_events, get_state_events_for_key, send_state_event},
	},
	events::{
		room::{
			canonical_alias::RoomCanonicalAliasEventContent,
			history_visibility::{HistoryVisibility, RoomHistoryVisibilityEventContent},
			join_rules::{JoinRule, RoomJoinRulesEventContent},
		},
		AnyStateEventContent, StateEventType,
	},
	serde::Raw,
	EventId, RoomId, UserId,
};
use service::Services;

use crate::{Ruma, RumaResponse};

/// # `PUT /_matrix/client/*/rooms/{roomId}/state/{eventType}/{stateKey}`
///
/// Sends a state event into the room.
///
/// - The only requirement for the content is that it has to be valid json
/// - Tries to send the event into the room, auth rules will determine if it is
///   allowed
/// - If event is new `canonical_alias`: Rejects if alias is incorrect
pub(crate) async fn send_state_event_for_key_route(
	State(services): State<crate::State>, body: Ruma<send_state_event::v3::Request>,
) -> Result<send_state_event::v3::Response> {
	let sender_user = body.sender_user.as_ref().expect("user is authenticated");

	Ok(send_state_event::v3::Response {
		event_id: send_state_event_for_key_helper(
			&services,
			sender_user,
			&body.room_id,
			&body.event_type,
			&body.body.body,
			body.state_key.clone(),
			if body.appservice_info.is_some() {
				body.timestamp
			} else {
				None
			},
		)
		.await?
		.into(),
	})
}

/// # `PUT /_matrix/client/*/rooms/{roomId}/state/{eventType}`
///
/// Sends a state event into the room.
///
/// - The only requirement for the content is that it has to be valid json
/// - Tries to send the event into the room, auth rules will determine if it is
///   allowed
/// - If event is new `canonical_alias`: Rejects if alias is incorrect
pub(crate) async fn send_state_event_for_empty_key_route(
	State(services): State<crate::State>, body: Ruma<send_state_event::v3::Request>,
) -> Result<RumaResponse<send_state_event::v3::Response>> {
	send_state_event_for_key_route(State(services), body)
		.await
		.map(RumaResponse)
}

/// # `GET /_matrix/client/v3/rooms/{roomid}/state`
///
/// Get all state events for a room.
///
/// - If not joined: Only works if current room history visibility is world
///   readable
pub(crate) async fn get_state_events_route(
	State(services): State<crate::State>, body: Ruma<get_state_events::v3::Request>,
) -> Result<get_state_events::v3::Response> {
	let sender_user = body.sender_user.as_ref().expect("user is authenticated");

	if !services
		.rooms
		.state_accessor
		.user_can_see_state_events(sender_user, &body.room_id)
		.await
	{
		return Err!(Request(Forbidden("You don't have permission to view the room state.")));
	}

	Ok(get_state_events::v3::Response {
		room_state: services
			.rooms
			.state_accessor
			.room_state_full(&body.room_id)
			.await?
			.values()
			.map(|pdu| pdu.to_state_event())
			.collect(),
	})
}

/// # `GET /_matrix/client/v3/rooms/{roomid}/state/{eventType}/{stateKey}`
///
/// Get single state event of a room with the specified state key.
/// The optional query parameter `?format=event|content` allows returning the
/// full room state event or just the state event's content (default behaviour)
///
/// - If not joined: Only works if current room history visibility is world
///   readable
pub(crate) async fn get_state_events_for_key_route(
	State(services): State<crate::State>, body: Ruma<get_state_events_for_key::v3::Request>,
) -> Result<get_state_events_for_key::v3::Response> {
	let sender_user = body.sender_user.as_ref().expect("user is authenticated");

	if !services
		.rooms
		.state_accessor
		.user_can_see_state_events(sender_user, &body.room_id)
		.await
	{
		return Err!(Request(Forbidden("You don't have permission to view the room state.")));
	}

	let event = services
		.rooms
		.state_accessor
		.room_state_get(&body.room_id, &body.event_type, &body.state_key)
		.await
		.map_err(|_| {
			err!(Request(NotFound(debug_warn!(
					room_id = ?body.room_id,
					event_type = ?body.event_type,
					"State event not found in room.",
			))))
		})?;

	let event_format = body
		.format
		.as_ref()
		.is_some_and(|f| f.to_lowercase().eq("event"));

	Ok(get_state_events_for_key::v3::Response {
		content: event_format.or(|| event.get_content_as_value()),
		event: event_format.then(|| event.to_state_event_value()),
	})
}

/// # `GET /_matrix/client/v3/rooms/{roomid}/state/{eventType}`
///
/// Get single state event of a room.
/// The optional query parameter `?format=event|content` allows returning the
/// full room state event or just the state event's content (default behaviour)
///
/// - If not joined: Only works if current room history visibility is world
///   readable
pub(crate) async fn get_state_events_for_empty_key_route(
	State(services): State<crate::State>, body: Ruma<get_state_events_for_key::v3::Request>,
) -> Result<RumaResponse<get_state_events_for_key::v3::Response>> {
	get_state_events_for_key_route(State(services), body)
		.await
		.map(RumaResponse)
}

async fn send_state_event_for_key_helper(
	services: &Services, sender: &UserId, room_id: &RoomId, event_type: &StateEventType,
	json: &Raw<AnyStateEventContent>, state_key: String, timestamp: Option<ruma::MilliSecondsSinceUnixEpoch>,
) -> Result<Arc<EventId>> {
	allowed_to_send_state_event(services, room_id, event_type, json).await?;
	let state_lock = services.rooms.state.mutex.lock(room_id).await;
	let event_id = services
		.rooms
		.timeline
		.build_and_append_pdu(
			PduBuilder {
				event_type: event_type.to_string().into(),
				content: serde_json::from_str(json.json().get()).expect("content is valid json"),
				unsigned: None,
				state_key: Some(state_key),
				redacts: None,
				timestamp,
			},
			sender,
			room_id,
			&state_lock,
		)
		.await?;

	Ok(event_id)
}

async fn allowed_to_send_state_event(
	services: &Services, room_id: &RoomId, event_type: &StateEventType, json: &Raw<AnyStateEventContent>,
) -> Result {
	match event_type {
		// Forbid m.room.encryption if encryption is disabled
		StateEventType::RoomEncryption => {
			if !services.globals.allow_encryption() {
				return Err(Error::BadRequest(ErrorKind::forbidden(), "Encryption has been disabled"));
			}
		},
		// admin room is a sensitive room, it should not ever be made public
		StateEventType::RoomJoinRules => {
			if let Ok(admin_room_id) = services.admin.get_admin_room().await {
				if admin_room_id == room_id {
					if let Ok(join_rule) = serde_json::from_str::<RoomJoinRulesEventContent>(json.json().get()) {
						if join_rule.join_rule == JoinRule::Public {
							return Err(Error::BadRequest(
								ErrorKind::forbidden(),
								"Admin room is not allowed to be public.",
							));
						}
					}
				}
			}
		},
		// admin room is a sensitive room, it should not ever be made world readable
		StateEventType::RoomHistoryVisibility => {
			if let Ok(admin_room_id) = services.admin.get_admin_room().await {
				if admin_room_id == room_id {
					if let Ok(visibility_content) =
						serde_json::from_str::<RoomHistoryVisibilityEventContent>(json.json().get())
					{
						if visibility_content.history_visibility == HistoryVisibility::WorldReadable {
							return Err(Error::BadRequest(
								ErrorKind::forbidden(),
								"Admin room is not allowed to be made world readable (public room history).",
							));
						}
					}
				}
			}
		},
		// TODO: allow alias if it previously existed
		StateEventType::RoomCanonicalAlias => {
			if let Ok(canonical_alias) = serde_json::from_str::<RoomCanonicalAliasEventContent>(json.json().get()) {
				let mut aliases = canonical_alias.alt_aliases.clone();

				if let Some(alias) = canonical_alias.alias {
					aliases.push(alias);
				}

				for alias in aliases {
					if !services.globals.server_is_ours(alias.server_name()) {
						return Err!(Request(Forbidden("canonical_alias must be for this server")));
					}

					if !services
						.rooms
						.alias
						.resolve_local_alias(&alias)
						.await
						.is_ok_and(|room| room == room_id)
					// Make sure it's the right room
					{
						return Err!(Request(Forbidden(
							"You are only allowed to send canonical_alias events when its aliases already exist"
						)));
					}
				}
			}
		},
		_ => (),
	}

	Ok(())
}
