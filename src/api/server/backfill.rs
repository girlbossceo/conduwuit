use std::cmp;

use axum::extract::State;
use conduit::{
	utils::{IterStream, ReadyExt},
	PduCount, Result,
};
use futures::{FutureExt, StreamExt};
use ruma::{api::federation::backfill::get_backfill, uint, MilliSecondsSinceUnixEpoch};

use super::AccessCheck;
use crate::Ruma;

/// # `GET /_matrix/federation/v1/backfill/<room_id>`
///
/// Retrieves events from before the sender joined the room, if the room's
/// history visibility allows.
pub(crate) async fn get_backfill_route(
	State(services): State<crate::State>, body: Ruma<get_backfill::v1::Request>,
) -> Result<get_backfill::v1::Response> {
	AccessCheck {
		services: &services,
		origin: body.origin(),
		room_id: &body.room_id,
		event_id: None,
	}
	.check()
	.await?;

	let until = body
		.v
		.iter()
		.stream()
		.filter_map(|event_id| {
			services
				.rooms
				.timeline
				.get_pdu_count(event_id)
				.map(Result::ok)
		})
		.ready_fold(PduCount::Backfilled(0), cmp::max)
		.await;

	let limit = body
		.limit
		.min(uint!(100))
		.try_into()
		.expect("UInt could not be converted to usize");

	let origin = body.origin();
	let pdus = services
		.rooms
		.timeline
		.pdus_rev(None, &body.room_id, Some(until))
		.await?
		.take(limit)
		.filter_map(|(_, pdu)| async move {
			if !services
				.rooms
				.state_accessor
				.server_can_see_event(origin, &pdu.room_id, &pdu.event_id)
				.await
			{
				return None;
			}

			services
				.rooms
				.timeline
				.get_pdu_json(&pdu.event_id)
				.await
				.ok()
		})
		.then(|pdu| services.sending.convert_to_outgoing_federation_event(pdu))
		.collect()
		.await;

	Ok(get_backfill::v1::Response {
		origin: services.globals.server_name().to_owned(),
		origin_server_ts: MilliSecondsSinceUnixEpoch::now(),
		pdus,
	})
}
