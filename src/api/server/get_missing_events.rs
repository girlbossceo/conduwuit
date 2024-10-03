use axum::extract::State;
use conduit::{Error, Result};
use ruma::{
	api::{client::error::ErrorKind, federation::event::get_missing_events},
	CanonicalJsonValue, EventId, RoomId,
};

use crate::Ruma;

/// # `POST /_matrix/federation/v1/get_missing_events/{roomId}`
///
/// Retrieves events that the sender is missing.
pub(crate) async fn get_missing_events_route(
	State(services): State<crate::State>, body: Ruma<get_missing_events::v1::Request>,
) -> Result<get_missing_events::v1::Response> {
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
		return Err(Error::BadRequest(ErrorKind::forbidden(), "Server is not in room"));
	}

	let limit = body
		.limit
		.try_into()
		.expect("UInt could not be converted to usize");

	let mut queued_events = body.latest_events.clone();
	// the vec will never have more entries the limit
	let mut events = Vec::with_capacity(limit);

	let mut i: usize = 0;
	while i < queued_events.len() && events.len() < limit {
		if let Ok(pdu) = services
			.rooms
			.timeline
			.get_pdu_json(&queued_events[i])
			.await
		{
			let room_id_str = pdu
				.get("room_id")
				.and_then(|val| val.as_str())
				.ok_or_else(|| Error::bad_database("Invalid event in database."))?;

			let event_room_id = <&RoomId>::try_from(room_id_str)
				.map_err(|_| Error::bad_database("Invalid room_id in event in database."))?;

			if event_room_id != body.room_id {
				return Err(Error::BadRequest(ErrorKind::InvalidParam, "Event from wrong room."));
			}

			if body.earliest_events.contains(&queued_events[i]) {
				i = i.saturating_add(1);
				continue;
			}

			if !services
				.rooms
				.state_accessor
				.server_can_see_event(origin, &body.room_id, &queued_events[i])
				.await?
			{
				i = i.saturating_add(1);
				continue;
			}

			let prev_events = pdu
				.get("prev_events")
				.and_then(CanonicalJsonValue::as_array)
				.unwrap_or_default();

			queued_events.extend(
				prev_events
					.iter()
					.map(<&EventId>::try_from)
					.filter_map(Result::ok)
					.map(ToOwned::to_owned),
			);

			events.push(
				services
					.sending
					.convert_to_outgoing_federation_event(pdu)
					.await,
			);
		}
		i = i.saturating_add(1);
	}

	Ok(get_missing_events::v1::Response {
		events,
	})
}
