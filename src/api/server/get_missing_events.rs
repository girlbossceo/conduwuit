use axum::extract::State;
use conduwuit::{
	Result, debug, debug_info, debug_warn,
	utils::{self},
	warn,
};
use ruma::api::federation::event::get_missing_events;

use super::AccessCheck;
use crate::Ruma;

/// arbitrary number but synapse's is 20 and we can handle lots of these anyways
const LIMIT_MAX: usize = 50;
/// spec says default is 10
const LIMIT_DEFAULT: usize = 10;

/// # `POST /_matrix/federation/v1/get_missing_events/{roomId}`
///
/// Retrieves events that the sender is missing.
pub(crate) async fn get_missing_events_route(
	State(services): State<crate::State>,
	body: Ruma<get_missing_events::v1::Request>,
) -> Result<get_missing_events::v1::Response> {
	AccessCheck {
		services: &services,
		origin: body.origin(),
		room_id: &body.room_id,
		event_id: None,
	}
	.check()
	.await?;

	let limit = body
		.limit
		.try_into()
		.unwrap_or(LIMIT_DEFAULT)
		.min(LIMIT_MAX);

	let mut queued_events = body.latest_events.clone();
	// the vec will never have more entries the limit
	let mut events = Vec::with_capacity(limit);

	let mut i: usize = 0;
	while i < queued_events.len() && events.len() < limit {
		let Ok(pdu) = services.rooms.timeline.get_pdu(&queued_events[i]).await else {
			debug_info!(?body.origin, "Event {} does not exist locally, skipping", &queued_events[i]);
			i = i.saturating_add(1);
			continue;
		};

		if pdu.room_id != body.room_id {
			warn!(?body.origin,
				"Got an event for the wrong room in database. Found {:?} in {:?}, server requested events in {:?}. Skipping.",
				pdu.event_id, pdu.room_id, body.room_id
			);
			i = i.saturating_add(1);
			continue;
		}

		if body.earliest_events.contains(&queued_events[i]) {
			i = i.saturating_add(1);
			continue;
		}

		if !services
			.rooms
			.state_accessor
			.server_can_see_event(body.origin(), &body.room_id, &queued_events[i])
			.await
		{
			debug!(?body.origin, "Server cannot see {:?} in {:?}, skipping", pdu.event_id, pdu.room_id);
			i = i.saturating_add(1);
			continue;
		}

		let Ok(pdu_json) = utils::to_canonical_object(&pdu) else {
			debug_warn!(?body.origin, "Failed to convert PDU in database to canonical JSON: {pdu:?}");
			i = i.saturating_add(1);
			continue;
		};

		queued_events.extend(pdu.prev_events.iter().map(ToOwned::to_owned));

		events.push(
			services
				.sending
				.convert_to_outgoing_federation_event(pdu_json)
				.await,
		);
	}

	Ok(get_missing_events::v1::Response { events })
}
