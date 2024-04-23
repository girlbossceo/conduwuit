use std::time::Duration;

use ruma::api::client::{
	error::ErrorKind,
	presence::{get_presence, set_presence},
};

use crate::{services, Error, Result, Ruma};

/// # `PUT /_matrix/client/r0/presence/{userId}/status`
///
/// Sets the presence state of the sender user.
pub(crate) async fn set_presence_route(body: Ruma<set_presence::v3::Request>) -> Result<set_presence::v3::Response> {
	if !services().globals.allow_local_presence() {
		return Err(Error::BadRequest(ErrorKind::forbidden(), "Presence is disabled on this server"));
	}

	let sender_user = body.sender_user.as_ref().expect("user is authenticated");
	services()
		.presence
		.set_presence(sender_user, &body.presence, None, None, body.status_msg.clone())?;

	Ok(set_presence::v3::Response {})
}

/// # `GET /_matrix/client/r0/presence/{userId}/status`
///
/// Gets the presence state of the given user.
///
/// - Only works if you share a room with the user
pub(crate) async fn get_presence_route(body: Ruma<get_presence::v3::Request>) -> Result<get_presence::v3::Response> {
	if !services().globals.allow_local_presence() {
		return Err(Error::BadRequest(ErrorKind::forbidden(), "Presence is disabled on this server"));
	}

	let sender_user = body.sender_user.as_ref().expect("user is authenticated");

	let mut presence_event = None;

	for _room_id in services()
		.rooms
		.user
		.get_shared_rooms(vec![sender_user.clone(), body.user_id.clone()])?
	{
		if let Some(presence) = services().presence.get_presence(sender_user)? {
			presence_event = Some(presence);
			break;
		}
	}

	if let Some(presence) = presence_event {
		Ok(get_presence::v3::Response {
			// TODO: Should ruma just use the presenceeventcontent type here?
			status_msg: presence.content.status_msg,
			currently_active: presence.content.currently_active,
			last_active_ago: presence
				.content
				.last_active_ago
				.map(|millis| Duration::from_millis(millis.into())),
			presence: presence.content.presence,
		})
	} else {
		Err(Error::BadRequest(
			ErrorKind::NotFound,
			"Presence state for this user was not found",
		))
	}
}
