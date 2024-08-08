use axum::extract::State;
use conduit::PduEvent;
use futures::StreamExt;
use ruma::{
	api::client::{error::ErrorKind, threads::get_threads},
	uint,
};

use crate::{Error, Result, Ruma};

/// # `GET /_matrix/client/r0/rooms/{roomId}/threads`
pub(crate) async fn get_threads_route(
	State(services): State<crate::State>, body: Ruma<get_threads::v1::Request>,
) -> Result<get_threads::v1::Response> {
	let sender_user = body.sender_user.as_ref().expect("user is authenticated");

	// Use limit or else 10, with maximum 100
	let limit = body
		.limit
		.unwrap_or_else(|| uint!(10))
		.try_into()
		.unwrap_or(10)
		.min(100);

	let from = if let Some(from) = &body.from {
		from.parse()
			.map_err(|_| Error::BadRequest(ErrorKind::InvalidParam, ""))?
	} else {
		u64::MAX
	};

	let room_id = &body.room_id;
	let threads: Vec<(u64, PduEvent)> = services
		.rooms
		.threads
		.threads_until(sender_user, &body.room_id, from, &body.include)
		.await?
		.take(limit)
		.filter_map(|(count, pdu)| async move {
			services
				.rooms
				.state_accessor
				.user_can_see_event(sender_user, room_id, &pdu.event_id)
				.await
				.then_some((count, pdu))
		})
		.collect()
		.await;

	let next_batch = threads.last().map(|(count, _)| count.to_string());

	Ok(get_threads::v1::Response {
		chunk: threads
			.into_iter()
			.map(|(_, pdu)| pdu.to_room_event())
			.collect(),
		next_batch,
	})
}
