mod data;

use std::sync::Arc;

use conduit::{Result, Server};
use data::Data;
use database::Database;
use ruma::{OwnedRoomId, RoomId};

pub struct Service {
	db: Data,
}

impl Service {
	pub fn build(_server: &Arc<Server>, db: &Arc<Database>) -> Result<Self> {
		Ok(Self {
			db: Data::new(db),
		})
	}

	/// Checks if a room exists.
	#[tracing::instrument(skip(self))]
	pub fn exists(&self, room_id: &RoomId) -> Result<bool> { self.db.exists(room_id) }

	#[must_use]
	pub fn iter_ids<'a>(&'a self) -> Box<dyn Iterator<Item = Result<OwnedRoomId>> + 'a> { self.db.iter_ids() }

	pub fn is_disabled(&self, room_id: &RoomId) -> Result<bool> { self.db.is_disabled(room_id) }

	pub fn disable_room(&self, room_id: &RoomId, disabled: bool) -> Result<()> {
		self.db.disable_room(room_id, disabled)
	}

	pub fn is_banned(&self, room_id: &RoomId) -> Result<bool> { self.db.is_banned(room_id) }

	pub fn ban_room(&self, room_id: &RoomId, banned: bool) -> Result<()> { self.db.ban_room(room_id, banned) }

	#[must_use]
	pub fn list_banned_rooms<'a>(&'a self) -> Box<dyn Iterator<Item = Result<OwnedRoomId>> + 'a> {
		self.db.list_banned_rooms()
	}
}
