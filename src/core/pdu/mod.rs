mod builder;
mod count;

use std::{cmp::Ordering, collections::BTreeMap, sync::Arc};

use ruma::{
	canonical_json::redact_content_in_place,
	events::{
		room::{member::RoomMemberEventContent, redaction::RoomRedactionEventContent},
		space::child::HierarchySpaceChildEvent,
		AnyEphemeralRoomEvent, AnyMessageLikeEvent, AnyStateEvent, AnyStrippedStateEvent, AnySyncStateEvent,
		AnySyncTimelineEvent, AnyTimelineEvent, StateEvent, TimelineEventType,
	},
	serde::Raw,
	state_res, CanonicalJsonObject, CanonicalJsonValue, EventId, MilliSecondsSinceUnixEpoch, OwnedEventId, OwnedRoomId,
	OwnedUserId, RoomId, RoomVersionId, UInt, UserId,
};
use serde::{Deserialize, Serialize};
use serde_json::{
	json,
	value::{to_raw_value, RawValue as RawJsonValue, Value as JsonValue},
};

pub use self::{
	builder::{Builder, Builder as PduBuilder},
	count::PduCount,
};
use crate::{err, is_true, warn, Error, Result};

#[derive(Deserialize)]
struct ExtractRedactedBecause {
	redacted_because: Option<serde::de::IgnoredAny>,
}

/// Content hashes of a PDU.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct EventHash {
	/// The SHA-256 hash.
	pub sha256: String,
}

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

impl PduEvent {
	#[tracing::instrument(skip(self), level = "debug")]
	pub fn redact(&mut self, room_version_id: RoomVersionId, reason: &Self) -> Result<()> {
		self.unsigned = None;

		let mut content = serde_json::from_str(self.content.get())
			.map_err(|_| Error::bad_database("PDU in db has invalid content."))?;

		redact_content_in_place(&mut content, &room_version_id, self.kind.to_string())
			.map_err(|e| Error::Redaction(self.sender.server_name().to_owned(), e))?;

		self.unsigned = Some(
			to_raw_value(&json!({
				"redacted_because": serde_json::to_value(reason).expect("to_value(PduEvent) always works")
			}))
			.expect("to string always works"),
		);

		self.content = to_raw_value(&content).expect("to string always works");

		Ok(())
	}

	#[must_use]
	pub fn is_redacted(&self) -> bool {
		let Some(unsigned) = &self.unsigned else {
			return false;
		};

		let Ok(unsigned) = ExtractRedactedBecause::deserialize(&**unsigned) else {
			return false;
		};

		unsigned.redacted_because.is_some()
	}

	pub fn remove_transaction_id(&mut self) -> Result<()> {
		let Some(unsigned) = &self.unsigned else {
			return Ok(());
		};

		let mut unsigned: BTreeMap<String, Box<RawJsonValue>> =
			serde_json::from_str(unsigned.get()).map_err(|e| err!(Database("Invalid unsigned in pdu event: {e}")))?;

		unsigned.remove("transaction_id");
		self.unsigned = to_raw_value(&unsigned)
			.map(Some)
			.expect("unsigned is valid");

		Ok(())
	}

	pub fn add_age(&mut self) -> Result<()> {
		let mut unsigned: BTreeMap<String, Box<RawJsonValue>> = self
			.unsigned
			.as_ref()
			.map_or_else(|| Ok(BTreeMap::new()), |u| serde_json::from_str(u.get()))
			.map_err(|e| err!(Database("Invalid unsigned in pdu event: {e}")))?;

		// deliberately allowing for the possibility of negative age
		let now: i128 = MilliSecondsSinceUnixEpoch::now().get().into();
		let then: i128 = self.origin_server_ts.into();
		let this_age = now.saturating_sub(then);

		unsigned.insert("age".to_owned(), to_raw_value(&this_age).expect("age is valid"));
		self.unsigned = to_raw_value(&unsigned)
			.map(Some)
			.expect("unsigned is valid");

		Ok(())
	}

	/// Copies the `redacts` property of the event to the `content` dict and
	/// vice-versa.
	///
	/// This follows the specification's
	/// [recommendation](https://spec.matrix.org/v1.10/rooms/v11/#moving-the-redacts-property-of-mroomredaction-events-to-a-content-property):
	///
	/// > For backwards-compatibility with older clients, servers should add a
	/// > redacts
	/// > property to the top level of m.room.redaction events in when serving
	/// > such events
	/// > over the Client-Server API.
	///
	/// > For improved compatibility with newer clients, servers should add a
	/// > redacts property
	/// > to the content of m.room.redaction events in older room versions when
	/// > serving
	/// > such events over the Client-Server API.
	#[must_use]
	pub fn copy_redacts(&self) -> (Option<Arc<EventId>>, Box<RawJsonValue>) {
		if self.kind == TimelineEventType::RoomRedaction {
			if let Ok(mut content) = serde_json::from_str::<RoomRedactionEventContent>(self.content.get()) {
				if let Some(redacts) = content.redacts {
					return (Some(redacts.into()), self.content.clone());
				} else if let Some(redacts) = self.redacts.clone() {
					content.redacts = Some(redacts.into());
					return (
						self.redacts.clone(),
						to_raw_value(&content).expect("Must be valid, we only added redacts field"),
					);
				}
			}
		}

		(self.redacts.clone(), self.content.clone())
	}

	#[must_use]
	pub fn get_content_as_value(&self) -> JsonValue {
		self.get_content()
			.expect("pdu content must be a valid JSON value")
	}

	pub fn get_content<T>(&self) -> Result<T>
	where
		T: for<'de> Deserialize<'de>,
	{
		serde_json::from_str(self.content.get())
			.map_err(|e| err!(Database("Failed to deserialize pdu content into type: {e}")))
	}

	pub fn contains_unsigned_property<F>(&self, property: &str, is_type: F) -> bool
	where
		F: FnOnce(&JsonValue) -> bool,
	{
		self.get_unsigned_as_value()
			.get(property)
			.map(is_type)
			.is_some_and(is_true!())
	}

	pub fn get_unsigned_property<T>(&self, property: &str) -> Result<T>
	where
		T: for<'de> Deserialize<'de>,
	{
		self.get_unsigned_as_value()
			.get_mut(property)
			.map(JsonValue::take)
			.map(serde_json::from_value)
			.ok_or(err!(Request(NotFound("property not found in unsigned object"))))?
			.map_err(|e| err!(Database("Failed to deserialize unsigned.{property} into type: {e}")))
	}

	#[must_use]
	pub fn get_unsigned_as_value(&self) -> JsonValue { self.get_unsigned::<JsonValue>().unwrap_or_default() }

	pub fn get_unsigned<T>(&self) -> Result<JsonValue> {
		self.unsigned
			.as_ref()
			.map(|raw| raw.get())
			.map(serde_json::from_str)
			.ok_or(err!(Request(NotFound("\"unsigned\" property not found in pdu"))))?
			.map_err(|e| err!(Database("Failed to deserialize \"unsigned\" into value: {e}")))
	}

	#[tracing::instrument(skip(self), level = "debug")]
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
	#[tracing::instrument(skip(self), level = "debug")]
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

	#[tracing::instrument(skip(self), level = "debug")]
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

	#[tracing::instrument(skip(self), level = "debug")]
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

	#[tracing::instrument(skip(self), level = "debug")]
	pub fn to_state_event(&self) -> Raw<AnyStateEvent> {
		serde_json::from_value(self.to_state_event_value()).expect("Raw::from_value always works")
	}

	#[tracing::instrument(skip(self), level = "debug")]
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

	#[tracing::instrument(skip(self), level = "debug")]
	pub fn to_stripped_state_event(&self) -> Raw<AnyStrippedStateEvent> {
		let json = json!({
			"content": self.content,
			"type": self.kind,
			"sender": self.sender,
			"state_key": self.state_key,
		});

		serde_json::from_value(json).expect("Raw::from_value always works")
	}

	#[tracing::instrument(skip(self), level = "debug")]
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

	#[tracing::instrument(skip(self), level = "debug")]
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

	pub fn from_id_val(event_id: &EventId, mut json: CanonicalJsonObject) -> Result<Self> {
		json.insert("event_id".into(), CanonicalJsonValue::String(event_id.into()));

		let value = serde_json::to_value(json)?;
		let pdu = serde_json::from_value(value)?;

		Ok(pdu)
	}
}

impl state_res::Event for PduEvent {
	type Id = Arc<EventId>;

	fn event_id(&self) -> &Self::Id { &self.event_id }

	fn room_id(&self) -> &RoomId { &self.room_id }

	fn sender(&self) -> &UserId { &self.sender }

	fn event_type(&self) -> &TimelineEventType { &self.kind }

	fn content(&self) -> &RawJsonValue { &self.content }

	fn origin_server_ts(&self) -> MilliSecondsSinceUnixEpoch { MilliSecondsSinceUnixEpoch(self.origin_server_ts) }

	fn state_key(&self) -> Option<&str> { self.state_key.as_deref() }

	fn prev_events(&self) -> impl DoubleEndedIterator<Item = &Self::Id> + Send + '_ { self.prev_events.iter() }

	fn auth_events(&self) -> impl DoubleEndedIterator<Item = &Self::Id> + Send + '_ { self.auth_events.iter() }

	fn redacts(&self) -> Option<&Self::Id> { self.redacts.as_ref() }
}

// These impl's allow us to dedup state snapshots when resolving state
// for incoming events (federation/send/{txn}).
impl Eq for PduEvent {}
impl PartialEq for PduEvent {
	fn eq(&self, other: &Self) -> bool { self.event_id == other.event_id }
}
impl PartialOrd for PduEvent {
	fn partial_cmp(&self, other: &Self) -> Option<Ordering> { Some(self.cmp(other)) }
}
impl Ord for PduEvent {
	fn cmp(&self, other: &Self) -> Ordering { self.event_id.cmp(&other.event_id) }
}

/// Generates a correct eventId for the incoming pdu.
///
/// Returns a tuple of the new `EventId` and the PDU as a `BTreeMap<String,
/// CanonicalJsonValue>`.
pub fn gen_event_id_canonical_json(
	pdu: &RawJsonValue, room_version_id: &RoomVersionId,
) -> Result<(OwnedEventId, CanonicalJsonObject)> {
	let value: CanonicalJsonObject = serde_json::from_str(pdu.get())
		.map_err(|e| err!(BadServerResponse(warn!("Error parsing incoming event: {e:?}"))))?;

	let event_id = gen_event_id(&value, room_version_id)?;

	Ok((event_id, value))
}

/// Generates a correct eventId for the incoming pdu.
pub fn gen_event_id(value: &CanonicalJsonObject, room_version_id: &RoomVersionId) -> Result<OwnedEventId> {
	let reference_hash = ruma::signatures::reference_hash(value, room_version_id)?;
	let event_id: OwnedEventId = format!("${reference_hash}").try_into()?;

	Ok(event_id)
}
