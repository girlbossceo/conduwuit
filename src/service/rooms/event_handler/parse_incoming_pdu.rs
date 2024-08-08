use conduit::{debug_warn, err, pdu::gen_event_id_canonical_json, Err, Result};
use ruma::{CanonicalJsonObject, OwnedEventId, OwnedRoomId, RoomId};
use serde_json::value::RawValue as RawJsonValue;

impl super::Service {
	pub async fn parse_incoming_pdu(
		&self, pdu: &RawJsonValue,
	) -> Result<(OwnedEventId, CanonicalJsonObject, OwnedRoomId)> {
		let value: CanonicalJsonObject = serde_json::from_str(pdu.get()).map_err(|e| {
			debug_warn!("Error parsing incoming event {pdu:#?}");
			err!(BadServerResponse("Error parsing incoming event {e:?}"))
		})?;

		let room_id: OwnedRoomId = value
			.get("room_id")
			.and_then(|id| RoomId::parse(id.as_str()?).ok())
			.ok_or(err!(Request(InvalidParam("Invalid room id in pdu"))))?;

		let Ok(room_version_id) = self.services.state.get_room_version(&room_id).await else {
			return Err!("Server is not in room {room_id}");
		};

		let Ok((event_id, value)) = gen_event_id_canonical_json(pdu, &room_version_id) else {
			// Event could not be converted to canonical json
			return Err!(Request(InvalidParam("Could not convert event to canonical json.")));
		};

		Ok((event_id, value, room_id))
	}
}
