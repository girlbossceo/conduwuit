use std::borrow::Borrow;

use axum::extract::State;
use conduit::{err, result::LogErr, utils::IterStream, Err, Result};
use futures::{FutureExt, StreamExt, TryStreamExt};
use ruma::api::federation::event::get_room_state;

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
		.map_err(|_| err!(Request(NotFound("PDU state not found."))))?;

	let pdus = services
		.rooms
		.state_accessor
		.state_full_ids(shortstatehash)
		.await
		.log_err()
		.map_err(|_| err!(Request(NotFound("PDU state IDs not found."))))?
		.values()
		.try_stream()
		.and_then(|id| services.rooms.timeline.get_pdu_json(id))
		.and_then(|pdu| {
			services
				.sending
				.convert_to_outgoing_federation_event(pdu)
				.map(Ok)
		})
		.try_collect()
		.await?;

	let auth_chain = services
		.rooms
		.auth_chain
		.event_ids_iter(&body.room_id, &[body.event_id.borrow()])
		.await?
		.map(Ok)
		.and_then(|id| async move { services.rooms.timeline.get_pdu_json(&id).await })
		.and_then(|pdu| {
			services
				.sending
				.convert_to_outgoing_federation_event(pdu)
				.map(Ok)
		})
		.try_collect()
		.await?;

	Ok(get_room_state::v1::Response {
		auth_chain,
		pdus,
	})
}
