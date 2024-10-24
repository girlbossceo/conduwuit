use std::cmp;

use axum::extract::State;
use conduit::{
	is_equal_to,
	utils::{IterStream, ReadyExt},
	Err, PduCount, Result,
};
use futures::{FutureExt, StreamExt};
use ruma::{api::federation::backfill::get_backfill, uint, user_id, MilliSecondsSinceUnixEpoch};

use crate::Ruma;

/// # `GET /_matrix/federation/v1/backfill/<room_id>`
///
/// Retrieves events from before the sender joined the room, if the room's
/// history visibility allows.
pub(crate) async fn get_backfill_route(
	State(services): State<crate::State>, body: Ruma<get_backfill::v1::Request>,
) -> Result<get_backfill::v1::Response> {
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
		return Err!(Request(Forbidden("Server is not in room.")));
	}

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

	let pdus = services
		.rooms
		.timeline
		.pdus_until(user_id!("@doesntmatter:conduit.rs"), &body.room_id, until)
		.await?
		.take(limit)
		.filter_map(|(_, pdu)| async move {
			if !services
				.rooms
				.state_accessor
				.server_can_see_event(origin, &pdu.room_id, &pdu.event_id)
				.await
				.is_ok_and(is_equal_to!(true))
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
