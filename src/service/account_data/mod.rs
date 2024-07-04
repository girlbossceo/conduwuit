mod data;

use std::{collections::HashMap, sync::Arc};

use conduit::Result;
use data::Data;
use ruma::{
	events::{AnyEphemeralRoomEvent, RoomAccountDataEventType},
	serde::Raw,
	RoomId, UserId,
};

pub struct Service {
	db: Data,
}

impl crate::Service for Service {
	fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
		Ok(Arc::new(Self {
			db: Data::new(args.db),
		}))
	}

	fn name(&self) -> &str { crate::service::make_name(std::module_path!()) }
}

impl Service {
	/// Places one event in the account data of the user and removes the
	/// previous entry.
	#[allow(clippy::needless_pass_by_value)]
	pub fn update(
		&self, room_id: Option<&RoomId>, user_id: &UserId, event_type: RoomAccountDataEventType,
		data: &serde_json::Value,
	) -> Result<()> {
		self.db.update(room_id, user_id, &event_type, data)
	}

	/// Searches the account data for a specific kind.
	#[allow(clippy::needless_pass_by_value)]
	pub fn get(
		&self, room_id: Option<&RoomId>, user_id: &UserId, event_type: RoomAccountDataEventType,
	) -> Result<Option<Box<serde_json::value::RawValue>>> {
		self.db.get(room_id, user_id, &event_type)
	}

	/// Returns all changes to the account data that happened after `since`.
	#[tracing::instrument(skip_all, name = "since")]
	pub fn changes_since(
		&self, room_id: Option<&RoomId>, user_id: &UserId, since: u64,
	) -> Result<HashMap<RoomAccountDataEventType, Raw<AnyEphemeralRoomEvent>>> {
		self.db.changes_since(room_id, user_id, since)
	}
}
