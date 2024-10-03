use conduit::{err, pdu::gen_event_id_canonical_json, result::FlatOk, Result};
use ruma::{CanonicalJsonObject, CanonicalJsonValue, OwnedEventId, OwnedRoomId, RoomId};
use serde_json::value::RawValue as RawJsonValue;

impl super::Service {
	pub async fn parse_incoming_pdu(
		&self, pdu: &RawJsonValue,
	) -> Result<(OwnedEventId, CanonicalJsonObject, OwnedRoomId)> {
		let value = serde_json::from_str::<CanonicalJsonObject>(pdu.get())
			.map_err(|e| err!(BadServerResponse(debug_warn!("Error parsing incoming event {e:?}"))))?;

		let room_id: OwnedRoomId = value
			.get("room_id")
			.and_then(CanonicalJsonValue::as_str)
			.map(RoomId::parse)
			.flat_ok_or(err!(Request(InvalidParam("Invalid room_id in pdu"))))?;

		let room_version_id = self
			.services
			.state
			.get_room_version(&room_id)
			.await
			.map_err(|_| err!("Server is not in room {room_id}"))?;

		let (event_id, value) = gen_event_id_canonical_json(pdu, &room_version_id)
			.map_err(|e| err!(Request(InvalidParam("Could not convert event to canonical json: {e}"))))?;

		Ok((event_id, value, room_id))
	}
}
