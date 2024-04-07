use ruma::{
	api::client::{error::ErrorKind, membership::mutual_rooms},
	OwnedRoomId,
};

use crate::{services, Error, Result, Ruma};

/// # `GET /_matrix/client/unstable/uk.half-shot.msc2666/user/mutual_rooms`
///
/// Gets all the rooms the sender shares with the specified user.
///
/// TODO: Implement pagination, currently this just returns everything
///
/// An implementation of [MSC2666](https://github.com/matrix-org/matrix-spec-proposals/pull/2666)
pub async fn get_mutual_rooms_route(
	body: Ruma<mutual_rooms::unstable::Request>,
) -> Result<mutual_rooms::unstable::Response> {
	let sender_user = body.sender_user.as_ref().expect("user is authenticated");

	if sender_user == &body.user_id {
		return Err(Error::BadRequest(
			ErrorKind::Unknown,
			"You cannot request rooms in common with yourself.",
		));
	}

	if !services().users.exists(&body.user_id)? {
		return Ok(mutual_rooms::unstable::Response {
			joined: vec![],
			next_batch_token: None,
		});
	}

	let mutual_rooms: Vec<OwnedRoomId> = services()
		.rooms
		.user
		.get_shared_rooms(vec![sender_user.clone(), body.user_id.clone()])?
		.filter_map(Result::ok)
		.collect();

	Ok(mutual_rooms::unstable::Response {
		joined: mutual_rooms,
		next_batch_token: None,
	})
}
