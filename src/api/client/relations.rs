use axum::extract::State;
use ruma::api::client::relations::{
	get_relating_events, get_relating_events_with_rel_type, get_relating_events_with_rel_type_and_event_type,
};

use crate::{Result, Ruma};

/// # `GET /_matrix/client/r0/rooms/{roomId}/relations/{eventId}/{relType}/{eventType}`
pub(crate) async fn get_relating_events_with_rel_type_and_event_type_route(
	State(services): State<crate::State>, body: Ruma<get_relating_events_with_rel_type_and_event_type::v1::Request>,
) -> Result<get_relating_events_with_rel_type_and_event_type::v1::Response> {
	let sender_user = body.sender_user.as_deref().expect("user is authenticated");

	let res = services
		.rooms
		.pdu_metadata
		.paginate_relations_with_filter(
			sender_user,
			&body.room_id,
			&body.event_id,
			body.event_type.clone().into(),
			body.rel_type.clone().into(),
			body.from.as_ref(),
			body.to.as_ref(),
			body.limit,
			body.recurse,
			body.dir,
		)
		.await?;

	Ok(get_relating_events_with_rel_type_and_event_type::v1::Response {
		chunk: res.chunk,
		next_batch: res.next_batch,
		prev_batch: res.prev_batch,
		recursion_depth: res.recursion_depth,
	})
}

/// # `GET /_matrix/client/r0/rooms/{roomId}/relations/{eventId}/{relType}`
pub(crate) async fn get_relating_events_with_rel_type_route(
	State(services): State<crate::State>, body: Ruma<get_relating_events_with_rel_type::v1::Request>,
) -> Result<get_relating_events_with_rel_type::v1::Response> {
	let sender_user = body.sender_user.as_deref().expect("user is authenticated");

	let res = services
		.rooms
		.pdu_metadata
		.paginate_relations_with_filter(
			sender_user,
			&body.room_id,
			&body.event_id,
			None,
			body.rel_type.clone().into(),
			body.from.as_ref(),
			body.to.as_ref(),
			body.limit,
			body.recurse,
			body.dir,
		)
		.await?;

	Ok(get_relating_events_with_rel_type::v1::Response {
		chunk: res.chunk,
		next_batch: res.next_batch,
		prev_batch: res.prev_batch,
		recursion_depth: res.recursion_depth,
	})
}

/// # `GET /_matrix/client/r0/rooms/{roomId}/relations/{eventId}`
pub(crate) async fn get_relating_events_route(
	State(services): State<crate::State>, body: Ruma<get_relating_events::v1::Request>,
) -> Result<get_relating_events::v1::Response> {
	let sender_user = body.sender_user.as_deref().expect("user is authenticated");

	services
		.rooms
		.pdu_metadata
		.paginate_relations_with_filter(
			sender_user,
			&body.room_id,
			&body.event_id,
			None,
			None,
			body.from.as_ref(),
			body.to.as_ref(),
			body.limit,
			body.recurse,
			body.dir,
		)
		.await
}
