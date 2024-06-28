mod data;

use std::sync::Arc;

use conduit::Server;
use data::Data;
use database::Database;
use ruma::{OwnedRoomId, RoomId};

use crate::Result;

pub struct Service {
	db: Data,
}

impl Service {
	pub fn build(_server: &Arc<Server>, db: &Arc<Database>) -> Result<Self> {
		Ok(Self {
			db: Data::new(db),
		})
	}

	#[tracing::instrument(skip(self))]
	pub fn set_public(&self, room_id: &RoomId) -> Result<()> { self.db.set_public(room_id) }

	#[tracing::instrument(skip(self))]
	pub fn set_not_public(&self, room_id: &RoomId) -> Result<()> { self.db.set_not_public(room_id) }

	#[tracing::instrument(skip(self))]
	pub fn is_public_room(&self, room_id: &RoomId) -> Result<bool> { self.db.is_public_room(room_id) }

	#[tracing::instrument(skip(self))]
	pub fn public_rooms(&self) -> impl Iterator<Item = Result<OwnedRoomId>> + '_ { self.db.public_rooms() }
}
