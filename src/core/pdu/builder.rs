use std::{collections::BTreeMap, sync::Arc};

use ruma::{events::TimelineEventType, EventId, MilliSecondsSinceUnixEpoch};
use serde::Deserialize;
use serde_json::value::RawValue as RawJsonValue;

/// Build the start of a PDU in order to add it to the Database.
#[derive(Debug, Deserialize)]
pub struct PduBuilder {
	#[serde(rename = "type")]
	pub event_type: TimelineEventType,
	pub content: Box<RawJsonValue>,
	pub unsigned: Option<BTreeMap<String, serde_json::Value>>,
	pub state_key: Option<String>,
	pub redacts: Option<Arc<EventId>>,
	/// For timestamped messaging, should only be used for appservices
	///
	/// Will be set to current time if None
	pub timestamp: Option<MilliSecondsSinceUnixEpoch>,
}
