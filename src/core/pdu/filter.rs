use ruma::api::client::filter::{RoomEventFilter, UrlFilter};
use serde_json::Value;

use crate::{implement, is_equal_to};

#[implement(super::PduEvent)]
#[must_use]
pub fn matches(&self, filter: &RoomEventFilter) -> bool {
	if !self.matches_sender(filter) {
		return false;
	}

	if !self.matches_room(filter) {
		return false;
	}

	if !self.matches_type(filter) {
		return false;
	}

	if !self.matches_url(filter) {
		return false;
	}

	true
}

#[implement(super::PduEvent)]
fn matches_room(&self, filter: &RoomEventFilter) -> bool {
	if filter.not_rooms.contains(&self.room_id) {
		return false;
	}

	if let Some(rooms) = filter.rooms.as_ref() {
		if !rooms.contains(&self.room_id) {
			return false;
		}
	}

	true
}

#[implement(super::PduEvent)]
fn matches_sender(&self, filter: &RoomEventFilter) -> bool {
	if filter.not_senders.contains(&self.sender) {
		return false;
	}

	if let Some(senders) = filter.senders.as_ref() {
		if !senders.contains(&self.sender) {
			return false;
		}
	}

	true
}

#[implement(super::PduEvent)]
fn matches_type(&self, filter: &RoomEventFilter) -> bool {
	let event_type = &self.kind.to_cow_str();
	if filter.not_types.iter().any(is_equal_to!(event_type)) {
		return false;
	}

	if let Some(types) = filter.types.as_ref() {
		if !types.iter().any(is_equal_to!(event_type)) {
			return false;
		}
	}

	true
}

#[implement(super::PduEvent)]
fn matches_url(&self, filter: &RoomEventFilter) -> bool {
	let Some(url_filter) = filter.url_filter.as_ref() else {
		return true;
	};

	//TODO: might be better to use Ruma's Raw rather than serde here
	let url = serde_json::from_str::<Value>(self.content.get())
		.expect("parsing content JSON failed")
		.get("url")
		.is_some_and(Value::is_string);

	match url_filter {
		UrlFilter::EventsWithUrl => url,
		UrlFilter::EventsWithoutUrl => !url,
	}
}
