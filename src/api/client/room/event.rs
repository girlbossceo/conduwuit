use axum::extract::State;
use conduit::{err, Result};
use futures::TryFutureExt;
use ruma::api::client::room::get_room_event;

use crate::Ruma;

/// # `GET /_matrix/client/r0/rooms/{roomId}/event/{eventId}`
///
/// Gets a single event.
///
/// - You have to currently be joined to the room (TODO: Respect history
///   visibility)
pub(crate) async fn get_room_event_route(
	State(services): State<crate::State>, ref body: Ruma<get_room_event::v3::Request>,
) -> Result<get_room_event::v3::Response> {
	Ok(get_room_event::v3::Response {
		event: services
			.rooms
			.timeline
			.get_pdu(&body.event_id)
			.map_err(|_| err!(Request(NotFound("Event {} not found.", &body.event_id))))
			.and_then(|event| async move {
				services
					.rooms
					.state_accessor
					.user_can_see_event(body.sender_user(), &event.room_id, &body.event_id)
					.await
					.then_some(event)
					.ok_or_else(|| err!(Request(Forbidden("You don't have permission to view this event."))))
			})
			.map_ok(|mut event| {
				event.add_age().ok();
				event.to_room_event()
			})
			.await?,
	})
}
