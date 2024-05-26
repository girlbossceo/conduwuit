use ruma::{OwnedRoomId, RoomId};

use crate::{utils, Error, KeyValueDatabase, Result};

pub trait Data: Send + Sync {
	/// Adds the room to the public room directory
	fn set_public(&self, room_id: &RoomId) -> Result<()>;

	/// Removes the room from the public room directory.
	fn set_not_public(&self, room_id: &RoomId) -> Result<()>;

	/// Returns true if the room is in the public room directory.
	fn is_public_room(&self, room_id: &RoomId) -> Result<bool>;

	/// Returns the unsorted public room directory
	fn public_rooms<'a>(&'a self) -> Box<dyn Iterator<Item = Result<OwnedRoomId>> + 'a>;
}

impl Data for KeyValueDatabase {
	fn set_public(&self, room_id: &RoomId) -> Result<()> { self.publicroomids.insert(room_id.as_bytes(), &[]) }

	fn set_not_public(&self, room_id: &RoomId) -> Result<()> { self.publicroomids.remove(room_id.as_bytes()) }

	fn is_public_room(&self, room_id: &RoomId) -> Result<bool> {
		Ok(self.publicroomids.get(room_id.as_bytes())?.is_some())
	}

	fn public_rooms<'a>(&'a self) -> Box<dyn Iterator<Item = Result<OwnedRoomId>> + 'a> {
		Box::new(self.publicroomids.iter().map(|(bytes, _)| {
			RoomId::parse(
				utils::string_from_bytes(&bytes)
					.map_err(|_| Error::bad_database("Room ID in publicroomids is invalid unicode."))?,
			)
			.map_err(|_| Error::bad_database("Room ID in publicroomids is invalid."))
		}))
	}
}
