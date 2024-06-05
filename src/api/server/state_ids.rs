use std::sync::Arc;

use ruma::api::{client::error::ErrorKind, federation::event::get_room_state_ids};

use crate::{services, Error, Result, Ruma};

/// # `GET /_matrix/federation/v1/state_ids/{roomId}`
///
/// Retrieves the current state of the room.
pub(crate) async fn get_room_state_ids_route(
	body: Ruma<get_room_state_ids::v1::Request>,
) -> Result<get_room_state_ids::v1::Response> {
	let origin = body.origin.as_ref().expect("server is authenticated");

	if !services()
		.rooms
		.state_cache
		.server_in_room(origin, &body.room_id)?
	{
		return Err(Error::BadRequest(ErrorKind::forbidden(), "Server is not in room."));
	}

	services()
		.rooms
		.event_handler
		.acl_check(origin, &body.room_id)?;

	let shortstatehash = services()
		.rooms
		.state_accessor
		.pdu_shortstatehash(&body.event_id)?
		.ok_or_else(|| Error::BadRequest(ErrorKind::NotFound, "Pdu state not found."))?;

	let pdu_ids = services()
		.rooms
		.state_accessor
		.state_full_ids(shortstatehash)
		.await?
		.into_values()
		.map(|id| (*id).to_owned())
		.collect();

	let auth_chain_ids = services()
		.rooms
		.auth_chain
		.event_ids_iter(&body.room_id, vec![Arc::from(&*body.event_id)])
		.await?;

	Ok(get_room_state_ids::v1::Response {
		auth_chain_ids: auth_chain_ids.map(|id| (*id).to_owned()).collect(),
		pdu_ids,
	})
}
