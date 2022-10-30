mod data;

pub use data::Data;
use ruma::{events::SyncEphemeralRoomEvent, RoomId, UserId};

use crate::Result;

pub struct Service {
    pub db: &'static dyn Data,
}

impl Service {
    /// Sets a user as typing until the timeout timestamp is reached or roomtyping_remove is
    /// called.
    pub fn typing_add(&self, user_id: &UserId, room_id: &RoomId, timeout: u64) -> Result<()> {
        self.db.typing_add(user_id, room_id, timeout)
    }

    /// Removes a user from typing before the timeout is reached.
    pub fn typing_remove(&self, user_id: &UserId, room_id: &RoomId) -> Result<()> {
        self.db.typing_remove(user_id, room_id)
    }

    /// Makes sure that typing events with old timestamps get removed.
    fn typings_maintain(&self, room_id: &RoomId) -> Result<()> {
        self.db.typings_maintain(room_id)
    }

    /// Returns the count of the last typing update in this room.
    pub fn last_typing_update(&self, room_id: &RoomId) -> Result<u64> {
        self.typings_maintain(room_id)?;

        self.db.last_typing_update(room_id)
    }

    /// Returns a new typing EDU.
    pub fn typings_all(
        &self,
        room_id: &RoomId,
    ) -> Result<SyncEphemeralRoomEvent<ruma::events::typing::TypingEventContent>> {
        let user_ids = self.db.typings_all(room_id)?;

        Ok(SyncEphemeralRoomEvent {
            content: ruma::events::typing::TypingEventContent {
                user_ids: user_ids.into_iter().collect(),
            },
        })
    }
}
