use axum::extract::State;
use ruma::api::{client::error::ErrorKind, federation::space::get_hierarchy};

use crate::{Error, Result, Ruma};

/// # `GET /_matrix/federation/v1/hierarchy/{roomId}`
///
/// Gets the space tree in a depth-first manner to locate child rooms of a given
/// space.
pub(crate) async fn get_hierarchy_route(
	State(services): State<crate::State>, body: Ruma<get_hierarchy::v1::Request>,
) -> Result<get_hierarchy::v1::Response> {
	let origin = body.origin.as_ref().expect("server is authenticated");

	if services.rooms.metadata.exists(&body.room_id).await {
		services
			.rooms
			.spaces
			.get_federation_hierarchy(&body.room_id, origin, body.suggested_only)
			.await
	} else {
		Err(Error::BadRequest(ErrorKind::NotFound, "Room does not exist."))
	}
}
