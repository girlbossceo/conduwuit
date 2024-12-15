use axum::extract::State;
use conduwuit::{err, Err, Event, Result};
use futures::{try_join, FutureExt, TryFutureExt};
use ruma::api::client::room::get_room_event;

use crate::{client::ignored_filter, Ruma};

/// # `GET /_matrix/client/r0/rooms/{roomId}/event/{eventId}`
///
/// Gets a single event.
pub(crate) async fn get_room_event_route(
	State(services): State<crate::State>, ref body: Ruma<get_room_event::v3::Request>,
) -> Result<get_room_event::v3::Response> {
	let event = services
		.rooms
		.timeline
		.get_pdu(&body.event_id)
		.map_err(|_| err!(Request(NotFound("Event {} not found.", &body.event_id))));

	let token = services
		.rooms
		.timeline
		.get_pdu_count(&body.event_id)
		.map_err(|_| err!(Request(NotFound("Event not found."))));

	let visible = services
		.rooms
		.state_accessor
		.user_can_see_event(body.sender_user(), &body.room_id, &body.event_id)
		.map(Ok);

	let (token, mut event, visible) = try_join!(token, event, visible)?;

	if !visible
		|| ignored_filter(&services, (token, event.clone()), body.sender_user())
			.await
			.is_none()
	{
		return Err!(Request(Forbidden("You don't have permission to view this event.")));
	}

	if event.event_id() != &body.event_id || event.room_id() != body.room_id {
		return Err!(Request(NotFound("Event not found")));
	}

	event.add_age().ok();

	let event = event.to_room_event();

	Ok(get_room_event::v3::Response {
		event,
	})
}
