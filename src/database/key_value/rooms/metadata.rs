use ruma::{OwnedRoomId, RoomId};
use tracing::error;

use crate::{database::KeyValueDatabase, service, services, utils, Error, Result};

impl service::rooms::metadata::Data for KeyValueDatabase {
    fn exists(&self, room_id: &RoomId) -> Result<bool> {
        let prefix = match services().rooms.short.get_shortroomid(room_id)? {
            Some(b) => b.to_be_bytes().to_vec(),
            None => return Ok(false),
        };

        // Look for PDUs in that room.
        Ok(self
            .pduid_pdu
            .iter_from(&prefix, false)
            .next()
            .filter(|(k, _)| k.starts_with(&prefix))
            .is_some())
    }

    fn iter_ids<'a>(&'a self) -> Box<dyn Iterator<Item = Result<OwnedRoomId>> + 'a> {
        Box::new(self.roomid_shortroomid.iter().map(|(bytes, _)| {
            RoomId::parse(
                utils::string_from_bytes(&bytes).map_err(|_| {
                    Error::bad_database("Room ID in publicroomids is invalid unicode.")
                })?,
            )
            .map_err(|_| Error::bad_database("Room ID in roomid_shortroomid is invalid."))
        }))
    }

    fn is_disabled(&self, room_id: &RoomId) -> Result<bool> {
        Ok(self.disabledroomids.get(room_id.as_bytes())?.is_some())
    }

    fn disable_room(&self, room_id: &RoomId, disabled: bool) -> Result<()> {
        if disabled {
            self.disabledroomids.insert(room_id.as_bytes(), &[])?;
        } else {
            self.disabledroomids.remove(room_id.as_bytes())?;
        }

        Ok(())
    }

    fn is_banned(&self, room_id: &RoomId) -> Result<bool> {
        Ok(self.bannedroomids.get(room_id.as_bytes())?.is_some())
    }

    fn ban_room(&self, room_id: &RoomId, banned: bool) -> Result<()> {
        if banned {
            self.bannedroomids.insert(room_id.as_bytes(), &[])?;
        } else {
            self.bannedroomids.remove(room_id.as_bytes())?;
        }

        Ok(())
    }

    fn list_banned_rooms<'a>(&'a self) -> Box<dyn Iterator<Item = Result<OwnedRoomId>> + 'a> {
        Box::new(self.bannedroomids.iter().map(
            |(room_id_bytes, _ /* non-banned rooms should not be in this table */)| {
                let room_id = utils::string_from_bytes(&room_id_bytes)
                    .map_err(|e| {
                        error!("Invalid room_id bytes in bannedroomids: {e}");
                        Error::bad_database("Invalid room_id in bannedroomids.")
                    })?
                    .try_into()
                    .map_err(|e| {
                        error!("Invalid room_id in bannedroomids: {e}");
                        Error::bad_database("Invalid room_id in bannedroomids")
                    })?;

                Ok(room_id)
            },
        ))
    }
}
