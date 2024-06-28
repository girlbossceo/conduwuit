mod data;

use std::sync::Arc;

use conduit::{Error, Result, Server};
use data::Data;
use database::Database;
use ruma::{
	api::client::error::ErrorKind,
	events::{
		room::power_levels::{RoomPowerLevels, RoomPowerLevelsEventContent},
		StateEventType,
	},
	OwnedRoomAliasId, OwnedRoomId, RoomAliasId, RoomId, UserId,
};

use crate::services;

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
	pub fn set_alias(&self, alias: &RoomAliasId, room_id: &RoomId, user_id: &UserId) -> Result<()> {
		if alias == services().globals.admin_alias && user_id != services().globals.server_user {
			Err(Error::BadRequest(
				ErrorKind::forbidden(),
				"Only the server user can set this alias",
			))
		} else {
			self.db.set_alias(alias, room_id, user_id)
		}
	}

	#[tracing::instrument(skip(self))]
	pub async fn remove_alias(&self, alias: &RoomAliasId, user_id: &UserId) -> Result<()> {
		if self.user_can_remove_alias(alias, user_id).await? {
			self.db.remove_alias(alias)
		} else {
			Err(Error::BadRequest(
				ErrorKind::forbidden(),
				"User is not permitted to remove this alias.",
			))
		}
	}

	#[tracing::instrument(skip(self))]
	pub fn resolve_local_alias(&self, alias: &RoomAliasId) -> Result<Option<OwnedRoomId>> {
		self.db.resolve_local_alias(alias)
	}

	#[tracing::instrument(skip(self))]
	pub fn local_aliases_for_room<'a>(
		&'a self, room_id: &RoomId,
	) -> Box<dyn Iterator<Item = Result<OwnedRoomAliasId>> + 'a> {
		self.db.local_aliases_for_room(room_id)
	}

	#[tracing::instrument(skip(self))]
	pub fn all_local_aliases<'a>(&'a self) -> Box<dyn Iterator<Item = Result<(OwnedRoomId, String)>> + 'a> {
		self.db.all_local_aliases()
	}

	async fn user_can_remove_alias(&self, alias: &RoomAliasId, user_id: &UserId) -> Result<bool> {
		let Some(room_id) = self.resolve_local_alias(alias)? else {
			return Err(Error::BadRequest(ErrorKind::NotFound, "Alias not found."));
		};

		let server_user = &services().globals.server_user;

		// The creator of an alias can remove it
		if self
            .db
            .who_created_alias(alias)?
            .is_some_and(|user| user == user_id)
            // Server admins can remove any local alias
            || services().admin.user_is_admin(user_id).await?
            // Always allow the server service account to remove the alias, since there may not be an admin room
            || server_user == user_id
		{
			Ok(true)
		// Checking whether the user is able to change canonical aliases of the
		// room
		} else if let Some(event) =
			services()
				.rooms
				.state_accessor
				.room_state_get(&room_id, &StateEventType::RoomPowerLevels, "")?
		{
			serde_json::from_str(event.content.get())
				.map_err(|_| Error::bad_database("Invalid event content for m.room.power_levels"))
				.map(|content: RoomPowerLevelsEventContent| {
					RoomPowerLevels::from(content).user_can_send_state(user_id, StateEventType::RoomCanonicalAlias)
				})
		// If there is no power levels event, only the room creator can change
		// canonical aliases
		} else if let Some(event) =
			services()
				.rooms
				.state_accessor
				.room_state_get(&room_id, &StateEventType::RoomCreate, "")?
		{
			Ok(event.sender == user_id)
		} else {
			Err(Error::bad_database("Room has no m.room.create event"))
		}
	}
}
