mod data;

pub(crate) use data::Data;
use ruma::{OwnedRoomId, RoomId};

use crate::Result;

pub(crate) struct Service {
	pub(crate) db: &'static dyn Data,
}

impl Service {
	/// Checks if a room exists.
	#[tracing::instrument(skip(self))]
	pub(crate) fn exists(&self, room_id: &RoomId) -> Result<bool> { self.db.exists(room_id) }

	pub(crate) fn iter_ids<'a>(&'a self) -> Box<dyn Iterator<Item = Result<OwnedRoomId>> + 'a> { self.db.iter_ids() }

	pub(crate) fn is_disabled(&self, room_id: &RoomId) -> Result<bool> { self.db.is_disabled(room_id) }

	pub(crate) fn disable_room(&self, room_id: &RoomId, disabled: bool) -> Result<()> {
		self.db.disable_room(room_id, disabled)
	}

	pub(crate) fn is_banned(&self, room_id: &RoomId) -> Result<bool> { self.db.is_banned(room_id) }

	pub(crate) fn ban_room(&self, room_id: &RoomId, banned: bool) -> Result<()> { self.db.ban_room(room_id, banned) }

	pub(crate) fn list_banned_rooms<'a>(&'a self) -> Box<dyn Iterator<Item = Result<OwnedRoomId>> + 'a> {
		self.db.list_banned_rooms()
	}
}
