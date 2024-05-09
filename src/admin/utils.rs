pub(crate) use conduit::utils::HtmlEscape;
use ruma::OwnedRoomId;

use crate::services;

pub(crate) fn escape_html(s: &str) -> String {
	s.replace('&', "&amp;")
		.replace('<', "&lt;")
		.replace('>', "&gt;")
}

pub(crate) fn get_room_info(id: &OwnedRoomId) -> (OwnedRoomId, u64, String) {
	(
		id.clone(),
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
