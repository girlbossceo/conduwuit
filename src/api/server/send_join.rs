#![allow(deprecated)]

use std::collections::BTreeMap;

use axum::extract::State;
use conduit::{pdu::gen_event_id_canonical_json, warn, Error, Result};
use ruma::{
	api::{client::error::ErrorKind, federation::membership::create_join_event},
	events::{
		room::member::{MembershipState, RoomMemberEventContent},
		StateEventType,
	},
	CanonicalJsonValue, OwnedServerName, OwnedUserId, RoomId, ServerName,
};
use serde_json::value::{to_raw_value, RawValue as RawJsonValue};
use service::Services;
use tokio::sync::RwLock;

use crate::Ruma;

/// helper method for /send_join v1 and v2
async fn create_join_event(
	services: &Services, origin: &ServerName, room_id: &RoomId, pdu: &RawJsonValue,
) -> Result<create_join_event::v1::RoomState> {
	if !services.rooms.metadata.exists(room_id)? {
		return Err(Error::BadRequest(ErrorKind::NotFound, "Room is unknown to this server."));
	}

	// ACL check origin server
	services.rooms.event_handler.acl_check(origin, room_id)?;

	// We need to return the state prior to joining, let's keep a reference to that
	// here
	let shortstatehash = services
		.rooms
		.state
		.get_room_shortstatehash(room_id)?
		.ok_or_else(|| Error::BadRequest(ErrorKind::NotFound, "Event state not found."))?;

	let pub_key_map = RwLock::new(BTreeMap::new());
	// let mut auth_cache = EventMap::new();

	// We do not add the event_id field to the pdu here because of signature and
	// hashes checks
	let room_version_id = services.rooms.state.get_room_version(room_id)?;

	let Ok((event_id, mut value)) = gen_event_id_canonical_json(pdu, &room_version_id) else {
		// Event could not be converted to canonical json
		return Err(Error::BadRequest(
			ErrorKind::InvalidParam,
			"Could not convert event to canonical json.",
		));
	};

	let event_type: StateEventType = serde_json::from_value(
		value
			.get("type")
			.ok_or_else(|| Error::BadRequest(ErrorKind::InvalidParam, "Event missing type property."))?
			.clone()
			.into(),
	)
	.map_err(|_| Error::BadRequest(ErrorKind::InvalidParam, "Event has invalid event type."))?;

	if event_type != StateEventType::RoomMember {
		return Err(Error::BadRequest(
			ErrorKind::InvalidParam,
			"Not allowed to send non-membership state event to join endpoint.",
		));
	}

	let content: RoomMemberEventContent = serde_json::from_value(
		value
			.get("content")
			.ok_or_else(|| Error::BadRequest(ErrorKind::InvalidParam, "Event missing content property"))?
			.clone()
			.into(),
	)
	.map_err(|_| Error::BadRequest(ErrorKind::InvalidParam, "Event content is empty or invalid"))?;

	if content.membership != MembershipState::Join {
		return Err(Error::BadRequest(
			ErrorKind::InvalidParam,
			"Not allowed to send a non-join membership event to join endpoint.",
		));
	}

	// ACL check sender server name
	let sender: OwnedUserId = serde_json::from_value(
		value
			.get("sender")
			.ok_or_else(|| Error::BadRequest(ErrorKind::InvalidParam, "Event missing sender property."))?
			.clone()
			.into(),
	)
	.map_err(|_| Error::BadRequest(ErrorKind::BadJson, "sender is not a valid user ID."))?;

	services
		.rooms
		.event_handler
		.acl_check(sender.server_name(), room_id)?;

	// check if origin server is trying to send for another server
	if sender.server_name() != origin {
		return Err(Error::BadRequest(
			ErrorKind::InvalidParam,
			"Not allowed to join on behalf of another server.",
		));
	}

	let state_key: OwnedUserId = serde_json::from_value(
		value
			.get("state_key")
			.ok_or_else(|| Error::BadRequest(ErrorKind::InvalidParam, "Event missing state_key property."))?
			.clone()
			.into(),
	)
	.map_err(|_| Error::BadRequest(ErrorKind::BadJson, "state_key is invalid or not a user ID."))?;

	if state_key != sender {
		return Err(Error::BadRequest(
			ErrorKind::InvalidParam,
			"State key does not match sender user",
		));
	};

	if content
		.join_authorized_via_users_server
		.is_some_and(|user| services.globals.user_is_local(&user))
		&& super::user_can_perform_restricted_join(services, &sender, room_id, &room_version_id).unwrap_or_default()
	{
		ruma::signatures::hash_and_sign_event(
			services.globals.server_name().as_str(),
			services.globals.keypair(),
			&mut value,
			&room_version_id,
		)
		.map_err(|_| Error::BadRequest(ErrorKind::InvalidParam, "Failed to sign event."))?;
	}

	services
		.server_keys
		.fetch_required_signing_keys([&value], &pub_key_map)
		.await?;

	let origin: OwnedServerName = serde_json::from_value(
		serde_json::to_value(
			value
				.get("origin")
				.ok_or_else(|| Error::BadRequest(ErrorKind::InvalidParam, "Event missing origin property."))?,
		)
		.expect("CanonicalJson is valid json value"),
	)
	.map_err(|_| Error::BadRequest(ErrorKind::InvalidParam, "origin is not a server name."))?;

	let mutex_lock = services
		.rooms
		.event_handler
		.mutex_federation
		.lock(room_id)
		.await;
	let pdu_id: Vec<u8> = services
		.rooms
		.event_handler
		.handle_incoming_pdu(&origin, room_id, &event_id, value.clone(), true, &pub_key_map)
		.await?
		.ok_or_else(|| Error::BadRequest(ErrorKind::InvalidParam, "Could not accept as timeline event."))?;
	drop(mutex_lock);

	let state_ids = services
		.rooms
		.state_accessor
		.state_full_ids(shortstatehash)
		.await?;
	let auth_chain_ids = services
		.rooms
		.auth_chain
		.event_ids_iter(room_id, state_ids.values().cloned().collect())
		.await?;

	services.sending.send_pdu_room(room_id, &pdu_id)?;

	Ok(create_join_event::v1::RoomState {
		auth_chain: auth_chain_ids
			.filter_map(|id| services.rooms.timeline.get_pdu_json(&id).ok().flatten())
			.map(|pdu| services.sending.convert_to_outgoing_federation_event(pdu))
			.collect(),
		state: state_ids
			.iter()
			.filter_map(|(_, id)| services.rooms.timeline.get_pdu_json(id).ok().flatten())
			.map(|pdu| services.sending.convert_to_outgoing_federation_event(pdu))
			.collect(),
		// Event field is required if the room version supports restricted join rules.
		event: Some(
			to_raw_value(&CanonicalJsonValue::Object(value))
				.expect("To raw json should not fail since only change was adding signature"),
		),
	})
}

/// # `PUT /_matrix/federation/v1/send_join/{roomId}/{eventId}`
///
/// Submits a signed join event.
pub(crate) async fn create_join_event_v1_route(
	State(services): State<crate::State>, body: Ruma<create_join_event::v1::Request>,
) -> Result<create_join_event::v1::Response> {
	let origin = body.origin.as_ref().expect("server is authenticated");

	if services
		.globals
		.config
		.forbidden_remote_server_names
		.contains(origin)
	{
		warn!(
			"Server {origin} tried joining room ID {} who has a server name that is globally forbidden. Rejecting.",
			&body.room_id,
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
			warn!(
				"Server {origin} tried joining room ID {} which has a server name that is globally forbidden. \
				 Rejecting.",
				&body.room_id,
			);
			return Err(Error::BadRequest(
				ErrorKind::forbidden(),
				"Server is banned on this homeserver.",
			));
		}
	}

	let room_state = create_join_event(&services, origin, &body.room_id, &body.pdu).await?;

	Ok(create_join_event::v1::Response {
		room_state,
	})
}

/// # `PUT /_matrix/federation/v2/send_join/{roomId}/{eventId}`
///
/// Submits a signed join event.
pub(crate) async fn create_join_event_v2_route(
	State(services): State<crate::State>, body: Ruma<create_join_event::v2::Request>,
) -> Result<create_join_event::v2::Response> {
	let origin = body.origin.as_ref().expect("server is authenticated");

	if services
		.globals
		.config
		.forbidden_remote_server_names
		.contains(origin)
	{
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

	let create_join_event::v1::RoomState {
		auth_chain,
		state,
		event,
	} = create_join_event(&services, origin, &body.room_id, &body.pdu).await?;
	let room_state = create_join_event::v2::RoomState {
		members_omitted: false,
		auth_chain,
		state,
		event,
		servers_in_room: None,
	};

	Ok(create_join_event::v2::Response {
		room_state,
	})
}
