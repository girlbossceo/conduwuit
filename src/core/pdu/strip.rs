use ruma::{
	events::{
		room::member::RoomMemberEventContent, space::child::HierarchySpaceChildEvent,
		AnyEphemeralRoomEvent, AnyMessageLikeEvent, AnyStateEvent, AnyStrippedStateEvent,
		AnySyncStateEvent, AnySyncTimelineEvent, AnyTimelineEvent, StateEvent,
	},
	serde::Raw,
};
use serde_json::{json, value::Value as JsonValue};

use crate::implement;

#[must_use]
#[implement(super::Pdu)]
pub fn to_sync_room_event(&self) -> Raw<AnySyncTimelineEvent> {
	let (redacts, content) = self.copy_redacts();
	let mut json = json!({
		"content": content,
		"type": self.kind,
		"event_id": self.event_id,
		"sender": self.sender,
		"origin_server_ts": self.origin_server_ts,
	});

	if let Some(unsigned) = &self.unsigned {
		json["unsigned"] = json!(unsigned);
	}
	if let Some(state_key) = &self.state_key {
		json["state_key"] = json!(state_key);
	}
	if let Some(redacts) = &redacts {
		json["redacts"] = json!(redacts);
	}

	serde_json::from_value(json).expect("Raw::from_value always works")
}

/// This only works for events that are also AnyRoomEvents.
#[must_use]
#[implement(super::Pdu)]
pub fn to_any_event(&self) -> Raw<AnyEphemeralRoomEvent> {
	let (redacts, content) = self.copy_redacts();
	let mut json = json!({
		"content": content,
		"type": self.kind,
		"event_id": self.event_id,
		"sender": self.sender,
		"origin_server_ts": self.origin_server_ts,
		"room_id": self.room_id,
	});

	if let Some(unsigned) = &self.unsigned {
		json["unsigned"] = json!(unsigned);
	}
	if let Some(state_key) = &self.state_key {
		json["state_key"] = json!(state_key);
	}
	if let Some(redacts) = &redacts {
		json["redacts"] = json!(redacts);
	}

	serde_json::from_value(json).expect("Raw::from_value always works")
}

#[must_use]
#[implement(super::Pdu)]
pub fn to_room_event(&self) -> Raw<AnyTimelineEvent> {
	let (redacts, content) = self.copy_redacts();
	let mut json = json!({
		"content": content,
		"type": self.kind,
		"event_id": self.event_id,
		"sender": self.sender,
		"origin_server_ts": self.origin_server_ts,
		"room_id": self.room_id,
	});

	if let Some(unsigned) = &self.unsigned {
		json["unsigned"] = json!(unsigned);
	}
	if let Some(state_key) = &self.state_key {
		json["state_key"] = json!(state_key);
	}
	if let Some(redacts) = &redacts {
		json["redacts"] = json!(redacts);
	}

	serde_json::from_value(json).expect("Raw::from_value always works")
}

#[must_use]
#[implement(super::Pdu)]
pub fn to_message_like_event(&self) -> Raw<AnyMessageLikeEvent> {
	let (redacts, content) = self.copy_redacts();
	let mut json = json!({
		"content": content,
		"type": self.kind,
		"event_id": self.event_id,
		"sender": self.sender,
		"origin_server_ts": self.origin_server_ts,
		"room_id": self.room_id,
	});

	if let Some(unsigned) = &self.unsigned {
		json["unsigned"] = json!(unsigned);
	}
	if let Some(state_key) = &self.state_key {
		json["state_key"] = json!(state_key);
	}
	if let Some(redacts) = &redacts {
		json["redacts"] = json!(redacts);
	}

	serde_json::from_value(json).expect("Raw::from_value always works")
}

#[must_use]
#[implement(super::Pdu)]
pub fn to_state_event_value(&self) -> JsonValue {
	let mut json = json!({
		"content": self.content,
		"type": self.kind,
		"event_id": self.event_id,
		"sender": self.sender,
		"origin_server_ts": self.origin_server_ts,
		"room_id": self.room_id,
		"state_key": self.state_key,
	});

	if let Some(unsigned) = &self.unsigned {
		json["unsigned"] = json!(unsigned);
	}

	json
}

#[must_use]
#[implement(super::Pdu)]
pub fn to_state_event(&self) -> Raw<AnyStateEvent> {
	serde_json::from_value(self.to_state_event_value()).expect("Raw::from_value always works")
}

#[must_use]
#[implement(super::Pdu)]
pub fn to_sync_state_event(&self) -> Raw<AnySyncStateEvent> {
	let mut json = json!({
		"content": self.content,
		"type": self.kind,
		"event_id": self.event_id,
		"sender": self.sender,
		"origin_server_ts": self.origin_server_ts,
		"state_key": self.state_key,
	});

	if let Some(unsigned) = &self.unsigned {
		json["unsigned"] = json!(unsigned);
	}

	serde_json::from_value(json).expect("Raw::from_value always works")
}

#[must_use]
#[implement(super::Pdu)]
pub fn to_stripped_state_event(&self) -> Raw<AnyStrippedStateEvent> {
	let json = json!({
		"content": self.content,
		"type": self.kind,
		"sender": self.sender,
		"state_key": self.state_key,
	});

	serde_json::from_value(json).expect("Raw::from_value always works")
}

#[must_use]
#[implement(super::Pdu)]
pub fn to_stripped_spacechild_state_event(&self) -> Raw<HierarchySpaceChildEvent> {
	let json = json!({
		"content": self.content,
		"type": self.kind,
		"sender": self.sender,
		"state_key": self.state_key,
		"origin_server_ts": self.origin_server_ts,
	});

	serde_json::from_value(json).expect("Raw::from_value always works")
}

#[must_use]
#[implement(super::Pdu)]
pub fn to_member_event(&self) -> Raw<StateEvent<RoomMemberEventContent>> {
	let mut json = json!({
		"content": self.content,
		"type": self.kind,
		"event_id": self.event_id,
		"sender": self.sender,
		"origin_server_ts": self.origin_server_ts,
		"redacts": self.redacts,
		"room_id": self.room_id,
		"state_key": self.state_key,
	});

	if let Some(unsigned) = &self.unsigned {
		json["unsigned"] = json!(unsigned);
	}

	serde_json::from_value(json).expect("Raw::from_value always works")
}
