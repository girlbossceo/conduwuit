mod builder;
mod content;
mod count;
mod event;
mod event_id;
mod filter;
mod id;
mod raw_id;
mod redact;
mod relation;
mod strip;
mod tests;
mod unsigned;

use std::{cmp::Ordering, sync::Arc};

use ruma::{
	events::TimelineEventType, CanonicalJsonObject, CanonicalJsonValue, EventId, OwnedRoomId, OwnedUserId, UInt,
};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue as RawJsonValue;

pub use self::{
	builder::{Builder, Builder as PduBuilder},
	count::PduCount,
	event::Event,
	event_id::*,
	id::*,
	raw_id::*,
};
use crate::Result;

/// Persistent Data Unit (Event)
#[derive(Clone, Deserialize, Serialize, Debug)]
pub struct PduEvent {
	pub event_id: Arc<EventId>,
	pub room_id: OwnedRoomId,
	pub sender: OwnedUserId,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub origin: Option<String>,
	pub origin_server_ts: UInt,
	#[serde(rename = "type")]
	pub kind: TimelineEventType,
	pub content: Box<RawJsonValue>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub state_key: Option<String>,
	pub prev_events: Vec<Arc<EventId>>,
	pub depth: UInt,
	pub auth_events: Vec<Arc<EventId>>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub redacts: Option<Arc<EventId>>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub unsigned: Option<Box<RawJsonValue>>,
	pub hashes: EventHash,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	// BTreeMap<Box<ServerName>, BTreeMap<ServerSigningKeyId, String>>
	pub signatures: Option<Box<RawJsonValue>>,
}

/// Content hashes of a PDU.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct EventHash {
	/// The SHA-256 hash.
	pub sha256: String,
}

impl PduEvent {
	pub fn from_id_val(event_id: &EventId, mut json: CanonicalJsonObject) -> Result<Self> {
		let event_id = CanonicalJsonValue::String(event_id.into());
		json.insert("event_id".into(), event_id);
		serde_json::to_value(json)
			.and_then(serde_json::from_value)
			.map_err(Into::into)
	}
}

/// Prevent derived equality which wouldn't limit itself to event_id
impl Eq for PduEvent {}

/// Equality determined by the Pdu's ID, not the memory representations.
impl PartialEq for PduEvent {
	fn eq(&self, other: &Self) -> bool { self.event_id == other.event_id }
}

/// Ordering determined by the Pdu's ID, not the memory representations.
impl PartialOrd for PduEvent {
	fn partial_cmp(&self, other: &Self) -> Option<Ordering> { Some(self.cmp(other)) }
}

/// Ordering determined by the Pdu's ID, not the memory representations.
impl Ord for PduEvent {
	fn cmp(&self, other: &Self) -> Ordering { self.event_id.cmp(&other.event_id) }
}
