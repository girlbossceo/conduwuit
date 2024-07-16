use axum::extract::State;
use conduit::{Error, Result};
use ruma::{
	api::{client::error::ErrorKind, federation::event::get_event},
	MilliSecondsSinceUnixEpoch, RoomId,
};
use service::sending::convert_to_outgoing_federation_event;

use crate::Ruma;

/// # `GET /_matrix/federation/v1/event/{eventId}`
///
/// Retrieves a single event from the server.
///
/// - Only works if a user of this server is currently invited or joined the
///   room
pub(crate) async fn get_event_route(
	State(services): State<crate::State>, body: Ruma<get_event::v1::Request>,
) -> Result<get_event::v1::Response> {
	let origin = body.origin.as_ref().expect("server is authenticated");

	let event = services
		.rooms
		.timeline
		.get_pdu_json(&body.event_id)?
		.ok_or_else(|| Error::BadRequest(ErrorKind::NotFound, "Event not found."))?;

	let room_id_str = event
		.get("room_id")
		.and_then(|val| val.as_str())
		.ok_or_else(|| Error::bad_database("Invalid event in database."))?;

	let room_id =
		<&RoomId>::try_from(room_id_str).map_err(|_| Error::bad_database("Invalid room_id in event in database."))?;

	if !services.rooms.state_accessor.is_world_readable(room_id)?
		&& !services.rooms.state_cache.server_in_room(origin, room_id)?
	{
		return Err(Error::BadRequest(ErrorKind::forbidden(), "Server is not in room."));
	}

	if !services
		.rooms
		.state_accessor
		.server_can_see_event(origin, room_id, &body.event_id)?
	{
		return Err(Error::BadRequest(ErrorKind::forbidden(), "Server is not allowed to see event."));
	}

	Ok(get_event::v1::Response {
		origin: services.globals.server_name().to_owned(),
		origin_server_ts: MilliSecondsSinceUnixEpoch::now(),
		pdu: convert_to_outgoing_federation_event(event),
	})
}
