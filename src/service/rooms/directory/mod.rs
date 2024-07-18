mod data;

use std::sync::Arc;

use conduit::Result;
use ruma::{OwnedRoomId, RoomId};

use self::data::Data;

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
	#[tracing::instrument(skip(self), level = "debug")]
	pub fn set_public(&self, room_id: &RoomId) -> Result<()> { self.db.set_public(room_id) }

	#[tracing::instrument(skip(self), level = "debug")]
	pub fn set_not_public(&self, room_id: &RoomId) -> Result<()> { self.db.set_not_public(room_id) }

	#[tracing::instrument(skip(self), level = "debug")]
	pub fn is_public_room(&self, room_id: &RoomId) -> Result<bool> { self.db.is_public_room(room_id) }

	#[tracing::instrument(skip(self), level = "debug")]
	pub fn public_rooms(&self) -> impl Iterator<Item = Result<OwnedRoomId>> + '_ { self.db.public_rooms() }
}
