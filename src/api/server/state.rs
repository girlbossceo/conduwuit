use std::sync::Arc;

use axum::extract::State;
use conduit::{Error, Result};
use ruma::api::{client::error::ErrorKind, federation::event::get_room_state};
use service::sending::convert_to_outgoing_federation_event;

use crate::Ruma;

/// # `GET /_matrix/federation/v1/state/{roomId}`
///
/// Retrieves a snapshot of a room's state at a given event.
pub(crate) async fn get_room_state_route(
	State(services): State<crate::State>, body: Ruma<get_room_state::v1::Request>,
) -> Result<get_room_state::v1::Response> {
	let origin = body.origin.as_ref().expect("server is authenticated");

	services
		.rooms
		.event_handler
		.acl_check(origin, &body.room_id)?;

	if !services
		.rooms
		.state_accessor
		.is_world_readable(&body.room_id)?
		&& !services
			.rooms
			.state_cache
			.server_in_room(origin, &body.room_id)?
	{
		return Err(Error::BadRequest(ErrorKind::forbidden(), "Server is not in room."));
	}

	let shortstatehash = services
		.rooms
		.state_accessor
		.pdu_shortstatehash(&body.event_id)?
		.ok_or_else(|| Error::BadRequest(ErrorKind::NotFound, "Pdu state not found."))?;

	let pdus = services
		.rooms
		.state_accessor
		.state_full_ids(shortstatehash)
		.await?
		.into_values()
		.map(|id| convert_to_outgoing_federation_event(services.rooms.timeline.get_pdu_json(&id).unwrap().unwrap()))
		.collect();

	let auth_chain_ids = services
		.rooms
		.auth_chain
		.event_ids_iter(&body.room_id, vec![Arc::from(&*body.event_id)])
		.await?;

	Ok(get_room_state::v1::Response {
		auth_chain: auth_chain_ids
			.filter_map(|id| {
				services
					.rooms
					.timeline
					.get_pdu_json(&id)
					.ok()?
					.map(convert_to_outgoing_federation_event)
			})
			.collect(),
		pdus,
	})
}
