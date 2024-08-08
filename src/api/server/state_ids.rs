use std::sync::Arc;

use axum::extract::State;
use conduit::{err, Err};
use futures::StreamExt;
use ruma::api::federation::event::get_room_state_ids;

use crate::{Result, Ruma};

/// # `GET /_matrix/federation/v1/state_ids/{roomId}`
///
/// Retrieves a snapshot of a room's state at a given event, in the form of
/// event IDs.
pub(crate) async fn get_room_state_ids_route(
	State(services): State<crate::State>, body: Ruma<get_room_state_ids::v1::Request>,
) -> Result<get_room_state_ids::v1::Response> {
	let origin = body.origin.as_ref().expect("server is authenticated");

	services
		.rooms
		.event_handler
		.acl_check(origin, &body.room_id)
		.await?;

	if !services
		.rooms
		.state_accessor
		.is_world_readable(&body.room_id)
		.await && !services
		.rooms
		.state_cache
		.server_in_room(origin, &body.room_id)
		.await
	{
		return Err!(Request(Forbidden("Server is not in room.")));
	}

	let shortstatehash = services
		.rooms
		.state_accessor
		.pdu_shortstatehash(&body.event_id)
		.await
		.map_err(|_| err!(Request(NotFound("Pdu state not found."))))?;

	let pdu_ids = services
		.rooms
		.state_accessor
		.state_full_ids(shortstatehash)
		.await
		.map_err(|_| err!(Request(NotFound("State ids not found"))))?
		.into_values()
		.map(|id| (*id).to_owned())
		.collect();

	let auth_chain_ids = services
		.rooms
		.auth_chain
		.event_ids_iter(&body.room_id, vec![Arc::from(&*body.event_id)])
		.await?
		.map(|id| (*id).to_owned())
		.collect()
		.await;

	Ok(get_room_state_ids::v1::Response {
		auth_chain_ids,
		pdu_ids,
	})
}
