mod data;

use std::sync::Arc;

use conduit::Result;
use data::Data;
use ruma::{OwnedRoomId, RoomId};

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
	/// Checks if a room exists.
	#[inline]
	pub fn exists(&self, room_id: &RoomId) -> Result<bool> { self.db.exists(room_id) }

	#[must_use]
	pub fn iter_ids<'a>(&'a self) -> Box<dyn Iterator<Item = Result<OwnedRoomId>> + 'a> { self.db.iter_ids() }

	#[inline]
	pub fn is_disabled(&self, room_id: &RoomId) -> Result<bool> { self.db.is_disabled(room_id) }

	#[inline]
	pub fn disable_room(&self, room_id: &RoomId, disabled: bool) -> Result<()> {
		self.db.disable_room(room_id, disabled)
	}

	#[inline]
	pub fn is_banned(&self, room_id: &RoomId) -> Result<bool> { self.db.is_banned(room_id) }

	#[inline]
	pub fn ban_room(&self, room_id: &RoomId, banned: bool) -> Result<()> { self.db.ban_room(room_id, banned) }

	#[inline]
	#[must_use]
	pub fn list_banned_rooms<'a>(&'a self) -> Box<dyn Iterator<Item = Result<OwnedRoomId>> + 'a> {
		self.db.list_banned_rooms()
	}
}
