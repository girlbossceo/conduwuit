mod data;

pub(crate) use data::Data;
use ruma::{OwnedRoomId, RoomId};

use crate::Result;

pub(crate) struct Service {
	pub(crate) db: &'static dyn Data,
}

impl Service {
	#[tracing::instrument(skip(self))]
	pub(crate) fn set_public(&self, room_id: &RoomId) -> Result<()> { self.db.set_public(room_id) }

	#[tracing::instrument(skip(self))]
	pub(crate) fn set_not_public(&self, room_id: &RoomId) -> Result<()> { self.db.set_not_public(room_id) }

	#[tracing::instrument(skip(self))]
	pub(crate) fn is_public_room(&self, room_id: &RoomId) -> Result<bool> { self.db.is_public_room(room_id) }

	#[tracing::instrument(skip(self))]
	pub(crate) fn public_rooms(&self) -> impl Iterator<Item = Result<OwnedRoomId>> + '_ { self.db.public_rooms() }
}
