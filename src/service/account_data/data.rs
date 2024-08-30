use std::{collections::HashMap, sync::Arc};

use conduit::{Error, Result};
use database::Map;
use ruma::{
	api::client::error::ErrorKind,
	events::{AnyGlobalAccountDataEvent, AnyRawAccountDataEvent, AnyRoomAccountDataEvent, RoomAccountDataEventType},
	serde::Raw,
	RoomId, UserId,
};

use crate::{globals, Dep};

pub(super) struct Data {
	roomuserdataid_accountdata: Arc<Map>,
	roomusertype_roomuserdataid: Arc<Map>,
	services: Services,
}

struct Services {
	globals: Dep<globals::Service>,
}

impl Data {
	pub(super) fn new(args: &crate::Args<'_>) -> Self {
		let db = &args.db;
		Self {
			roomuserdataid_accountdata: db["roomuserdataid_accountdata"].clone(),
			roomusertype_roomuserdataid: db["roomusertype_roomuserdataid"].clone(),
			services: Services {
				globals: args.depend::<globals::Service>("globals"),
			},
		}
	}

	/// Places one event in the account data of the user and removes the
	/// previous entry.
	pub(super) fn update(
		&self, room_id: Option<&RoomId>, user_id: &UserId, event_type: &RoomAccountDataEventType,
		data: &serde_json::Value,
	) -> Result<()> {
		let mut prefix = room_id
			.map(ToString::to_string)
			.unwrap_or_default()
			.as_bytes()
			.to_vec();
		prefix.push(0xFF);
		prefix.extend_from_slice(user_id.as_bytes());
		prefix.push(0xFF);

		let mut roomuserdataid = prefix.clone();
		roomuserdataid.extend_from_slice(&self.services.globals.next_count()?.to_be_bytes());
		roomuserdataid.push(0xFF);
		roomuserdataid.extend_from_slice(event_type.to_string().as_bytes());

		let mut key = prefix;
		key.extend_from_slice(event_type.to_string().as_bytes());

		if data.get("type").is_none() || data.get("content").is_none() {
			return Err(Error::BadRequest(
				ErrorKind::InvalidParam,
				"Account data doesn't have all required fields.",
			));
		}

		self.roomuserdataid_accountdata.insert(
			&roomuserdataid,
			&serde_json::to_vec(&data).expect("to_vec always works on json values"),
		)?;

		let prev = self.roomusertype_roomuserdataid.get(&key)?;

		self.roomusertype_roomuserdataid
			.insert(&key, &roomuserdataid)?;

		// Remove old entry
		if let Some(prev) = prev {
			self.roomuserdataid_accountdata.remove(&prev)?;
		}

		Ok(())
	}

	/// Searches the account data for a specific kind.
	pub(super) fn get(
		&self, room_id: Option<&RoomId>, user_id: &UserId, kind: &RoomAccountDataEventType,
	) -> Result<Option<Box<serde_json::value::RawValue>>> {
		let mut key = room_id
			.map(ToString::to_string)
			.unwrap_or_default()
			.as_bytes()
			.to_vec();
		key.push(0xFF);
		key.extend_from_slice(user_id.as_bytes());
		key.push(0xFF);
		key.extend_from_slice(kind.to_string().as_bytes());

		self.roomusertype_roomuserdataid
			.get(&key)?
			.and_then(|roomuserdataid| {
				self.roomuserdataid_accountdata
					.get(&roomuserdataid)
					.transpose()
			})
			.transpose()?
			.map(|data| serde_json::from_slice(&data).map_err(|_| Error::bad_database("could not deserialize")))
			.transpose()
	}

	/// Returns all changes to the account data that happened after `since`.
	pub(super) fn changes_since(
		&self, room_id: Option<&RoomId>, user_id: &UserId, since: u64,
	) -> Result<Vec<AnyRawAccountDataEvent>> {
		let mut userdata = HashMap::new();

		let mut prefix = room_id
			.map(ToString::to_string)
			.unwrap_or_default()
			.as_bytes()
			.to_vec();
		prefix.push(0xFF);
		prefix.extend_from_slice(user_id.as_bytes());
		prefix.push(0xFF);

		// Skip the data that's exactly at since, because we sent that last time
		let mut first_possible = prefix.clone();
		first_possible.extend_from_slice(&(since.saturating_add(1)).to_be_bytes());

		for r in self
			.roomuserdataid_accountdata
			.iter_from(&first_possible, false)
			.take_while(move |(k, _)| k.starts_with(&prefix))
			.map(|(k, v)| {
				Ok::<_, Error>((
					k,
					match room_id {
						None => serde_json::from_slice::<Raw<AnyGlobalAccountDataEvent>>(&v)
							.map(AnyRawAccountDataEvent::Global)
							.map_err(|_| Error::bad_database("Database contains invalid account data."))?,
						Some(_) => serde_json::from_slice::<Raw<AnyRoomAccountDataEvent>>(&v)
							.map(AnyRawAccountDataEvent::Room)
							.map_err(|_| Error::bad_database("Database contains invalid account data."))?,
					},
				))
			}) {
			let (kind, data) = r?;
			userdata.insert(kind, data);
		}

		Ok(userdata.into_values().collect())
	}
}
