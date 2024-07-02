use std::sync::Arc;

use conduit::{utils, Error, Result};
use database::{Database, Map};
use ruma::{api::client::error::ErrorKind, OwnedRoomAliasId, OwnedRoomId, OwnedUserId, RoomAliasId, RoomId, UserId};

use crate::services;

pub(super) struct Data {
	alias_userid: Arc<Map>,
	alias_roomid: Arc<Map>,
	aliasid_alias: Arc<Map>,
}

impl Data {
	pub(super) fn new(db: &Arc<Database>) -> Self {
		Self {
			alias_userid: db["alias_userid"].clone(),
			alias_roomid: db["alias_roomid"].clone(),
			aliasid_alias: db["aliasid_alias"].clone(),
		}
	}

	pub(super) fn set_alias(&self, alias: &RoomAliasId, room_id: &RoomId, user_id: &UserId) -> Result<()> {
		// Comes first as we don't want a stuck alias
		self.alias_userid
			.insert(alias.alias().as_bytes(), user_id.as_bytes())?;

		self.alias_roomid
			.insert(alias.alias().as_bytes(), room_id.as_bytes())?;

		let mut aliasid = room_id.as_bytes().to_vec();
		aliasid.push(0xFF);
		aliasid.extend_from_slice(&services().globals.next_count()?.to_be_bytes());
		self.aliasid_alias.insert(&aliasid, alias.as_bytes())?;

		Ok(())
	}

	pub(super) fn remove_alias(&self, alias: &RoomAliasId) -> Result<()> {
		if let Some(room_id) = self.alias_roomid.get(alias.alias().as_bytes())? {
			let mut prefix = room_id.to_vec();
			prefix.push(0xFF);

			for (key, _) in self.aliasid_alias.scan_prefix(prefix) {
				self.aliasid_alias.remove(&key)?;
			}

			self.alias_roomid.remove(alias.alias().as_bytes())?;

			self.alias_userid.remove(alias.alias().as_bytes())?;
		} else {
			return Err(Error::BadRequest(ErrorKind::NotFound, "Alias does not exist or is invalid."));
		}

		Ok(())
	}

	pub(super) fn resolve_local_alias(&self, alias: &RoomAliasId) -> Result<Option<OwnedRoomId>> {
		self.alias_roomid
			.get(alias.alias().as_bytes())?
			.map(|bytes| {
				RoomId::parse(
					utils::string_from_bytes(&bytes)
						.map_err(|_| Error::bad_database("Room ID in alias_roomid is invalid unicode."))?,
				)
				.map_err(|_| Error::bad_database("Room ID in alias_roomid is invalid."))
			})
			.transpose()
	}

	pub(super) fn who_created_alias(&self, alias: &RoomAliasId) -> Result<Option<OwnedUserId>> {
		self.alias_userid
			.get(alias.alias().as_bytes())?
			.map(|bytes| {
				UserId::parse(
					utils::string_from_bytes(&bytes)
						.map_err(|_| Error::bad_database("User ID in alias_userid is invalid unicode."))?,
				)
				.map_err(|_| Error::bad_database("User ID in alias_roomid is invalid."))
			})
			.transpose()
	}

	pub(super) fn local_aliases_for_room<'a>(
		&'a self, room_id: &RoomId,
	) -> Box<dyn Iterator<Item = Result<OwnedRoomAliasId>> + 'a> {
		let mut prefix = room_id.as_bytes().to_vec();
		prefix.push(0xFF);

		Box::new(self.aliasid_alias.scan_prefix(prefix).map(|(_, bytes)| {
			utils::string_from_bytes(&bytes)
				.map_err(|_| Error::bad_database("Invalid alias bytes in aliasid_alias."))?
				.try_into()
				.map_err(|_| Error::bad_database("Invalid alias in aliasid_alias."))
		}))
	}

	pub(super) fn all_local_aliases<'a>(&'a self) -> Box<dyn Iterator<Item = Result<(OwnedRoomId, String)>> + 'a> {
		Box::new(
			self.alias_roomid
				.iter()
				.map(|(room_alias_bytes, room_id_bytes)| {
					let room_alias_localpart = utils::string_from_bytes(&room_alias_bytes)
						.map_err(|_| Error::bad_database("Invalid alias bytes in aliasid_alias."))?;

					let room_id = utils::string_from_bytes(&room_id_bytes)
						.map_err(|_| Error::bad_database("Invalid room_id bytes in aliasid_alias."))?
						.try_into()
						.map_err(|_| Error::bad_database("Invalid room_id in aliasid_alias."))?;

					Ok((room_id, room_alias_localpart))
				}),
		)
	}
}
