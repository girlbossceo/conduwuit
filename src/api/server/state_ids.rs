use std::{borrow::Borrow, iter::once};

use axum::extract::State;
use conduwuit::{err, Result};
use futures::StreamExt;
use ruma::{api::federation::event::get_room_state_ids, OwnedEventId};

use super::AccessCheck;
use crate::Ruma;

/// # `GET /_matrix/federation/v1/state_ids/{roomId}`
///
/// Retrieves a snapshot of a room's state at a given event, in the form of
/// event IDs.
pub(crate) async fn get_room_state_ids_route(
	State(services): State<crate::State>,
	body: Ruma<get_room_state_ids::v1::Request>,
) -> Result<get_room_state_ids::v1::Response> {
	AccessCheck {
		services: &services,
		origin: body.origin(),
		room_id: &body.room_id,
		event_id: None,
	}
	.check()
	.await?;

	let shortstatehash = services
		.rooms
		.state_accessor
		.pdu_shortstatehash(&body.event_id)
		.await
		.map_err(|_| err!(Request(NotFound("Pdu state not found."))))?;

	let pdu_ids: Vec<OwnedEventId> = services
		.rooms
		.state_accessor
		.state_full_ids(shortstatehash)
		.await
		.map_err(|_| err!(Request(NotFound("State ids not found"))))?
		.into_values()
		.collect();

	let auth_chain_ids = services
		.rooms
		.auth_chain
		.event_ids_iter(&body.room_id, once(body.event_id.borrow()))
		.await?
		.map(|id| (*id).to_owned())
		.collect()
		.await;

	Ok(get_room_state_ids::v1::Response { auth_chain_ids, pdu_ids })
}
