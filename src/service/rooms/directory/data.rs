use std::sync::Arc;

use database::KvTree;
use ruma::{OwnedRoomId, RoomId};

use crate::{utils, Error, KeyValueDatabase, Result};

pub(super) struct Data {
	publicroomids: Arc<dyn KvTree>,
}

impl Data {
	pub(super) fn new(db: &Arc<KeyValueDatabase>) -> Self {
		Self {
			publicroomids: db.publicroomids.clone(),
		}
	}

	pub(super) fn set_public(&self, room_id: &RoomId) -> Result<()> {
		self.publicroomids.insert(room_id.as_bytes(), &[])
	}

	pub(super) fn set_not_public(&self, room_id: &RoomId) -> Result<()> {
		self.publicroomids.remove(room_id.as_bytes())
	}

	pub(super) fn is_public_room(&self, room_id: &RoomId) -> Result<bool> {
		Ok(self.publicroomids.get(room_id.as_bytes())?.is_some())
	}

	pub(super) fn public_rooms<'a>(&'a self) -> Box<dyn Iterator<Item = Result<OwnedRoomId>> + 'a> {
		Box::new(self.publicroomids.iter().map(|(bytes, _)| {
			RoomId::parse(
				utils::string_from_bytes(&bytes)
					.map_err(|_| Error::bad_database("Room ID in publicroomids is invalid unicode."))?,
			)
			.map_err(|_| Error::bad_database("Room ID in publicroomids is invalid."))
		}))
	}
}
