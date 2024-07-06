mod data;
mod remote;

use std::sync::Arc;

use conduit::{Error, Result};
use data::Data;
use ruma::{
	api::{appservice, client::error::ErrorKind},
	events::{
		room::power_levels::{RoomPowerLevels, RoomPowerLevelsEventContent},
		StateEventType,
	},
	OwnedRoomAliasId, OwnedRoomId, OwnedServerName, RoomAliasId, RoomId, RoomOrAliasId, UserId,
};

use crate::{appservice::RegistrationInfo, server_is_ours, services};

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

	pub async fn resolve(&self, room: &RoomOrAliasId) -> Result<OwnedRoomId> {
		if room.is_room_id() {
			let room_id: &RoomId = &RoomId::parse(room).expect("valid RoomId");
			Ok(room_id.to_owned())
		} else {
			let alias: &RoomAliasId = &RoomAliasId::parse(room).expect("valid RoomAliasId");
			Ok(self.resolve_alias(alias, None).await?.0)
		}
	}

	#[tracing::instrument(skip(self), name = "resolve")]
	pub async fn resolve_alias(
		&self, room_alias: &RoomAliasId, servers: Option<&Vec<OwnedServerName>>,
	) -> Result<(OwnedRoomId, Option<Vec<OwnedServerName>>)> {
		if !server_is_ours(room_alias.server_name())
			&& (!servers
				.as_ref()
				.is_some_and(|servers| servers.contains(&services().globals.server_name().to_owned()))
				|| servers.as_ref().is_none())
		{
			return remote::resolve(room_alias, servers).await;
		}

		let room_id: Option<OwnedRoomId> = match self.resolve_local_alias(room_alias)? {
			Some(r) => Some(r),
			None => self.resolve_appservice_alias(room_alias).await?,
		};

		room_id.map_or_else(
			|| Err(Error::BadRequest(ErrorKind::NotFound, "Room with alias not found.")),
			|room_id| Ok((room_id, None)),
		)
	}

	#[tracing::instrument(skip(self), level = "debug")]
	pub fn resolve_local_alias(&self, alias: &RoomAliasId) -> Result<Option<OwnedRoomId>> {
		self.db.resolve_local_alias(alias)
	}

	#[tracing::instrument(skip(self), level = "debug")]
	pub fn local_aliases_for_room<'a>(
		&'a self, room_id: &RoomId,
	) -> Box<dyn Iterator<Item = Result<OwnedRoomAliasId>> + 'a + Send> {
		self.db.local_aliases_for_room(room_id)
	}

	#[tracing::instrument(skip(self), level = "debug")]
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

	async fn resolve_appservice_alias(&self, room_alias: &RoomAliasId) -> Result<Option<OwnedRoomId>> {
		for appservice in services().appservice.read().await.values() {
			if appservice.aliases.is_match(room_alias.as_str())
				&& matches!(
					services()
						.sending
						.send_appservice_request(
							appservice.registration.clone(),
							appservice::query::query_room_alias::v1::Request {
								room_alias: room_alias.to_owned(),
							},
						)
						.await,
					Ok(Some(_opt_result))
				) {
				return Ok(Some(
					services()
						.rooms
						.alias
						.resolve_local_alias(room_alias)?
						.ok_or_else(|| Error::bad_config("Room does not exist."))?,
				));
			}
		}

		Ok(None)
	}
}

pub async fn appservice_checks(room_alias: &RoomAliasId, appservice_info: &Option<RegistrationInfo>) -> Result<()> {
	if !server_is_ours(room_alias.server_name()) {
		return Err(Error::BadRequest(ErrorKind::InvalidParam, "Alias is from another server."));
	}

	if let Some(ref info) = appservice_info {
		if !info.aliases.is_match(room_alias.as_str()) {
			return Err(Error::BadRequest(ErrorKind::Exclusive, "Room alias is not in namespace."));
		}
	} else if services().appservice.is_exclusive_alias(room_alias).await {
		return Err(Error::BadRequest(ErrorKind::Exclusive, "Room alias reserved by appservice."));
	}

	Ok(())
}
