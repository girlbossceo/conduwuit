mod data;

use std::collections::HashMap;

pub(crate) use data::Data;
use ruma::{
	events::{AnyEphemeralRoomEvent, RoomAccountDataEventType},
	serde::Raw,
	RoomId, UserId,
};

use crate::Result;

pub(crate) struct Service {
	pub(crate) db: &'static dyn Data,
}

impl Service {
	/// Places one event in the account data of the user and removes the
	/// previous entry.
	#[tracing::instrument(skip(self, room_id, user_id, event_type, data))]
	pub(crate) fn update(
		&self, room_id: Option<&RoomId>, user_id: &UserId, event_type: RoomAccountDataEventType,
		data: &serde_json::Value,
	) -> Result<()> {
		self.db.update(room_id, user_id, event_type, data)
	}

	/// Searches the account data for a specific kind.
	#[tracing::instrument(skip(self, room_id, user_id, event_type))]
	pub(crate) fn get(
		&self, room_id: Option<&RoomId>, user_id: &UserId, event_type: RoomAccountDataEventType,
	) -> Result<Option<Box<serde_json::value::RawValue>>> {
		self.db.get(room_id, user_id, event_type)
	}

	/// Returns all changes to the account data that happened after `since`.
	#[tracing::instrument(skip(self, room_id, user_id, since))]
	pub(crate) fn changes_since(
		&self, room_id: Option<&RoomId>, user_id: &UserId, since: u64,
	) -> Result<HashMap<RoomAccountDataEventType, Raw<AnyEphemeralRoomEvent>>> {
		self.db.changes_since(room_id, user_id, since)
	}
}
