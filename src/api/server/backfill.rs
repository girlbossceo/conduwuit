use std::cmp;

use axum::extract::State;
use conduwuit::{
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
	State(services): State<crate::State>, ref body: Ruma<get_backfill::v1::Request>,
) -> Result<get_backfill::v1::Response> {
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
		.min(uint!(100))
		.try_into()
		.expect("UInt could not be converted to usize");

	let from = body
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
		.ready_fold(PduCount::min(), cmp::max)
		.await;

	Ok(get_backfill::v1::Response {
		origin_server_ts: MilliSecondsSinceUnixEpoch::now(),

		origin: services.globals.server_name().to_owned(),

		pdus: services
			.rooms
			.timeline
			.pdus_rev(None, &body.room_id, Some(from.saturating_add(1)))
			.await?
			.take(limit)
			.filter_map(|(_, pdu)| async move {
				services
					.rooms
					.state_accessor
					.server_can_see_event(body.origin(), &pdu.room_id, &pdu.event_id)
					.await
					.then_some(pdu)
			})
			.filter_map(|pdu| async move {
				services
					.rooms
					.timeline
					.get_pdu_json(&pdu.event_id)
					.await
					.ok()
			})
			.then(|pdu| services.sending.convert_to_outgoing_federation_event(pdu))
			.collect()
			.await,
	})
}
