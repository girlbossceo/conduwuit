use ruma::{
	canonical_json::redact_content_in_place,
	events::{room::redaction::RoomRedactionEventContent, TimelineEventType},
	OwnedEventId, RoomVersionId,
};
use serde::Deserialize;
use serde_json::{
	json,
	value::{to_raw_value, RawValue as RawJsonValue},
};

use crate::{implement, Error, Result};

#[derive(Deserialize)]
struct ExtractRedactedBecause {
	redacted_because: Option<serde::de::IgnoredAny>,
}

#[implement(super::Pdu)]
pub fn redact(&mut self, room_version_id: &RoomVersionId, reason: &Self) -> Result {
	self.unsigned = None;

	let mut content = serde_json::from_str(self.content.get())
		.map_err(|_| Error::bad_database("PDU in db has invalid content."))?;

	redact_content_in_place(&mut content, room_version_id, self.kind.to_string())
		.map_err(|e| Error::Redaction(self.sender.server_name().to_owned(), e))?;

	self.unsigned = Some(
		to_raw_value(&json!({
			"redacted_because": serde_json::to_value(reason).expect("to_value(Pdu) always works")
		}))
		.expect("to string always works"),
	);

	self.content = to_raw_value(&content).expect("to string always works");

	Ok(())
}

#[implement(super::Pdu)]
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
#[implement(super::Pdu)]
#[must_use]
pub fn copy_redacts(&self) -> (Option<OwnedEventId>, Box<RawJsonValue>) {
	if self.kind == TimelineEventType::RoomRedaction {
		if let Ok(mut content) =
			serde_json::from_str::<RoomRedactionEventContent>(self.content.get())
		{
			if let Some(redacts) = content.redacts {
				return (Some(redacts), self.content.clone());
			} else if let Some(redacts) = self.redacts.clone() {
				content.redacts = Some(redacts);
				return (
					self.redacts.clone(),
					to_raw_value(&content).expect("Must be valid, we only added redacts field"),
				);
			}
		}
	}

	(self.redacts.clone(), self.content.clone())
}
