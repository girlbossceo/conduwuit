use js_int::UInt;
use ruma_events::{
    collections::all::RoomEvent, stripped::AnyStrippedStateEvent, EventJson, EventType,
};
use ruma_federation_api::EventHash;
use ruma_identifiers::{EventId, RoomId, UserId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Deserialize, Serialize)]
pub struct PduEvent {
    pub event_id: EventId,
    pub room_id: RoomId,
    pub sender: UserId,
    pub origin: String,
    pub origin_server_ts: UInt,
    #[serde(rename = "type")]
    pub kind: EventType,
    pub content: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_key: Option<String>,
    pub prev_events: Vec<EventId>,
    pub depth: UInt,
    pub auth_events: Vec<EventId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redacts: Option<EventId>,
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub unsigned: serde_json::Map<String, serde_json::Value>,
    pub hashes: EventHash,
    pub signatures: HashMap<String, HashMap<String, String>>,
}

impl PduEvent {
    pub fn to_room_event(&self) -> EventJson<RoomEvent> {
        // Can only fail in rare circumstances that won't ever happen here, see
        // https://docs.rs/serde_json/1.0.50/serde_json/fn.to_string.html
        let json = serde_json::to_string(&self).unwrap();
        // EventJson's deserialize implementation always returns `Ok(...)`
        serde_json::from_str::<EventJson<RoomEvent>>(&json).unwrap()
    }

    pub fn to_stripped_state_event(&self) -> EventJson<AnyStrippedStateEvent> {
        // Can only fail in rare circumstances that won't ever happen here, see
        // https://docs.rs/serde_json/1.0.50/serde_json/fn.to_string.html
        let json = serde_json::to_string(&self).unwrap();

        // EventJson's deserialize implementation always returns `Ok(...)`
        serde_json::from_str::<EventJson<AnyStrippedStateEvent>>(&json).unwrap()
    }
}
