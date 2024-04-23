use ruma::api::client::{error::ErrorKind, typing::create_typing_event};

use crate::{services, utils, Error, Result, Ruma};

/// # `PUT /_matrix/client/r0/rooms/{roomId}/typing/{userId}`
///
/// Sets the typing state of the sender user.
pub(crate) async fn create_typing_event_route(
	body: Ruma<create_typing_event::v3::Request>,
) -> Result<create_typing_event::v3::Response> {
	use create_typing_event::v3::Typing;

	let sender_user = body.sender_user.as_ref().expect("user is authenticated");

	if !services()
		.rooms
		.state_cache
		.is_joined(sender_user, &body.room_id)?
	{
		return Err(Error::BadRequest(ErrorKind::forbidden(), "You are not in this room."));
	}

	if let Typing::Yes(duration) = body.state {
		let duration = utils::clamp(
			duration.as_millis() as u64,
			services().globals.config.typing_client_timeout_min_s * 1000,
			services().globals.config.typing_client_timeout_max_s * 1000,
		);
		services()
			.rooms
			.typing
			.typing_add(sender_user, &body.room_id, utils::millis_since_unix_epoch() + duration)
			.await?;
	} else {
		services()
			.rooms
			.typing
			.typing_remove(sender_user, &body.room_id)
			.await?;
	}

	Ok(create_typing_event::v3::Response {})
}
