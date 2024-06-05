use ruma::{
	api::{client::error::ErrorKind, federation::backfill::get_backfill},
	uint, user_id, MilliSecondsSinceUnixEpoch,
};

use crate::{services, Error, PduEvent, Result, Ruma};

/// # `GET /_matrix/federation/v1/backfill/<room_id>`
///
/// Retrieves events from before the sender joined the room, if the room's
/// history visibility allows.
pub(crate) async fn get_backfill_route(body: Ruma<get_backfill::v1::Request>) -> Result<get_backfill::v1::Response> {
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

	let until = body
		.v
		.iter()
		.map(|event_id| services().rooms.timeline.get_pdu_count(event_id))
		.filter_map(|r| r.ok().flatten())
		.max()
		.ok_or_else(|| Error::BadRequest(ErrorKind::InvalidParam, "Event not found."))?;

	let limit = body
		.limit
		.min(uint!(100))
		.try_into()
		.expect("UInt could not be converted to usize");

	let all_events = services()
		.rooms
		.timeline
		.pdus_until(user_id!("@doesntmatter:conduit.rs"), &body.room_id, until)?
		.take(limit);

	let events = all_events
		.filter_map(Result::ok)
		.filter(|(_, e)| {
			matches!(
				services()
					.rooms
					.state_accessor
					.server_can_see_event(origin, &e.room_id, &e.event_id,),
				Ok(true),
			)
		})
		.map(|(_, pdu)| services().rooms.timeline.get_pdu_json(&pdu.event_id))
		.filter_map(|r| r.ok().flatten())
		.map(PduEvent::convert_to_outgoing_federation_event)
		.collect();

	Ok(get_backfill::v1::Response {
		origin: services().globals.server_name().to_owned(),
		origin_server_ts: MilliSecondsSinceUnixEpoch::now(),
		pdus: events,
	})
}
