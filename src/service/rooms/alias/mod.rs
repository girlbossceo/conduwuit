mod data;

pub use data::Data;

use crate::Result;
use ruma::{OwnedRoomAliasId, OwnedRoomId, RoomAliasId, RoomId};

pub struct Service {
    pub db: &'static dyn Data,
}

impl Service {
    #[tracing::instrument(skip(self))]
    pub fn set_alias(&self, alias: &RoomAliasId, room_id: &RoomId) -> Result<()> {
        self.db.set_alias(alias, room_id)
    }

    #[tracing::instrument(skip(self))]
    pub fn remove_alias(&self, alias: &RoomAliasId) -> Result<()> {
        self.db.remove_alias(alias)
    }

    #[tracing::instrument(skip(self))]
    pub fn resolve_local_alias(&self, alias: &RoomAliasId) -> Result<Option<OwnedRoomId>> {
        self.db.resolve_local_alias(alias)
    }

    #[tracing::instrument(skip(self))]
    pub fn local_aliases_for_room<'a>(
        &'a self,
        room_id: &RoomId,
    ) -> Box<dyn Iterator<Item = Result<OwnedRoomAliasId>> + 'a> {
        self.db.local_aliases_for_room(room_id)
    }
}
