use ruma::{api::client::error::ErrorKind, CanonicalJsonObject, OwnedEventId, OwnedRoomId, RoomId};
use serde_json::value::RawValue as RawJsonValue;
use tracing::warn;

use crate::{service::pdu::gen_event_id_canonical_json, services, Error, Result};

pub fn parse_incoming_pdu(pdu: &RawJsonValue) -> Result<(OwnedEventId, CanonicalJsonObject, OwnedRoomId)> {
	let value: CanonicalJsonObject = serde_json::from_str(pdu.get()).map_err(|e| {
		warn!("Error parsing incoming event {:?}: {:?}", pdu, e);
		Error::BadServerResponse("Invalid PDU in server response")
	})?;

	let room_id: OwnedRoomId = value
		.get("room_id")
		.and_then(|id| RoomId::parse(id.as_str()?).ok())
		.ok_or(Error::BadRequest(ErrorKind::InvalidParam, "Invalid room id in pdu"))?;

	let Ok(room_version_id) = services().rooms.state.get_room_version(&room_id) else {
		return Err(Error::Err(format!("Server is not in room {room_id}")));
	};

	let Ok((event_id, value)) = gen_event_id_canonical_json(pdu, &room_version_id) else {
		// Event could not be converted to canonical json
		return Err(Error::BadRequest(
			ErrorKind::InvalidParam,
			"Could not convert event to canonical json.",
		));
	};

	Ok((event_id, value, room_id))
}
