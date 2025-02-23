use axum::extract::State;
use conduwuit::{Result, err};
use ruma::{MilliSecondsSinceUnixEpoch, RoomId, api::federation::event::get_event};

use super::AccessCheck;
use crate::Ruma;

/// # `GET /_matrix/federation/v1/event/{eventId}`
///
/// Retrieves a single event from the server.
///
/// - Only works if a user of this server is currently invited or joined the
///   room
pub(crate) async fn get_event_route(
	State(services): State<crate::State>,
	body: Ruma<get_event::v1::Request>,
) -> Result<get_event::v1::Response> {
	let event = services
		.rooms
		.timeline
		.get_pdu_json(&body.event_id)
		.await
		.map_err(|_| err!(Request(NotFound("Event not found."))))?;

	let room_id: &RoomId = event
		.get("room_id")
		.and_then(|val| val.as_str())
		.ok_or_else(|| err!(Database("Invalid event in database.")))?
		.try_into()
		.map_err(|_| err!(Database("Invalid room_id in event in database.")))?;

	AccessCheck {
		services: &services,
		origin: body.origin(),
		room_id,
		event_id: Some(&body.event_id),
	}
	.check()
	.await?;

	Ok(get_event::v1::Response {
		origin: services.globals.server_name().to_owned(),
		origin_server_ts: MilliSecondsSinceUnixEpoch::now(),
		pdu: services
			.sending
			.convert_to_outgoing_federation_event(event)
			.await,
	})
}
