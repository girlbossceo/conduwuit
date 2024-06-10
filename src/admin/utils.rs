pub(crate) use conduit::utils::HtmlEscape;
use conduit_core::Error;
use ruma::{OwnedRoomId, OwnedUserId, RoomId, UserId};
use service::user_is_local;

use crate::{services, Result};

pub(crate) fn escape_html(s: &str) -> String {
	s.replace('&', "&amp;")
		.replace('<', "&lt;")
		.replace('>', "&gt;")
}

pub(crate) fn get_room_info(id: &RoomId) -> (OwnedRoomId, u64, String) {
	(
		id.into(),
		services()
			.rooms
			.state_cache
			.room_joined_count(id)
			.ok()
			.flatten()
			.unwrap_or(0),
		services()
			.rooms
			.state_accessor
			.get_name(id)
			.ok()
			.flatten()
			.unwrap_or_else(|| id.to_string()),
	)
}

/// Parses user ID
pub(crate) fn parse_user_id(user_id: &str) -> Result<OwnedUserId> {
	UserId::parse_with_server_name(user_id.to_lowercase(), services().globals.server_name())
		.map_err(|e| Error::Err(format!("The supplied username is not a valid username: {e}")))
}

/// Parses user ID as our local user
pub(crate) fn parse_local_user_id(user_id: &str) -> Result<OwnedUserId> {
	let user_id = parse_user_id(user_id)?;

	if !user_is_local(&user_id) {
		return Err(Error::Err(String::from("User does not belong to our server.")));
	}

	Ok(user_id)
}

/// Parses user ID that is an active (not guest or deactivated) local user
pub(crate) fn parse_active_local_user_id(user_id: &str) -> Result<OwnedUserId> {
	let user_id = parse_local_user_id(user_id)?;

	if !services().users.exists(&user_id)? {
		return Err(Error::Err(String::from("User does not exist on this server.")));
	}

	if services().users.is_deactivated(&user_id)? {
		return Err(Error::Err(String::from("User is deactivated.")));
	}

	Ok(user_id)
}
