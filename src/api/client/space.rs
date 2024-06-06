use std::str::FromStr;

use ruma::{
	api::client::{error::ErrorKind, space::get_hierarchy},
	uint, UInt,
};

use crate::{service::rooms::spaces::PagnationToken, services, Error, Result, Ruma};

/// # `GET /_matrix/client/v1/rooms/{room_id}/hierarchy`
///
/// Paginates over the space tree in a depth-first manner to locate child rooms
/// of a given space.
pub(crate) async fn get_hierarchy_route(body: Ruma<get_hierarchy::v1::Request>) -> Result<get_hierarchy::v1::Response> {
	let sender_user = body.sender_user.as_ref().expect("user is authenticated");

	let limit: usize = body
		.limit
		.unwrap_or_else(|| uint!(10))
		.try_into()
		.unwrap_or(10)
		.min(100);

	let max_depth = body
		.max_depth
		.unwrap_or_else(|| UInt::from(3_u32))
		.min(UInt::from(10_u32));

	let key = body
		.from
		.as_ref()
		.and_then(|s| PagnationToken::from_str(s).ok());

	// Should prevent unexpeded behaviour in (bad) clients
	if let Some(ref token) = key {
		if token.suggested_only != body.suggested_only || token.max_depth != max_depth {
			return Err(Error::BadRequest(
				ErrorKind::InvalidParam,
				"suggested_only and max_depth cannot change on paginated requests",
			));
		}
	}

	services()
		.rooms
		.spaces
		.get_client_hierarchy(
			sender_user,
			&body.room_id,
			limit,
			key.map_or(0, |token| token.skip.try_into().unwrap_or(0)),
			max_depth.try_into().unwrap_or(3),
			body.suggested_only,
		)
		.await
}
