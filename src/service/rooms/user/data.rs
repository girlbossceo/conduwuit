use std::sync::Arc;

use conduit::{utils, Error, Result};
use database::Map;
use ruma::{OwnedRoomId, OwnedUserId, RoomId, UserId};

use crate::{globals, rooms, Dep};

pub(super) struct Data {
	userroomid_notificationcount: Arc<Map>,
	userroomid_highlightcount: Arc<Map>,
	roomuserid_lastnotificationread: Arc<Map>,
	roomsynctoken_shortstatehash: Arc<Map>,
	userroomid_joined: Arc<Map>,
	services: Services,
}

struct Services {
	globals: Dep<globals::Service>,
	short: Dep<rooms::short::Service>,
}

impl Data {
	pub(super) fn new(args: &crate::Args<'_>) -> Self {
		let db = &args.db;
		Self {
			userroomid_notificationcount: db["userroomid_notificationcount"].clone(),
			userroomid_highlightcount: db["userroomid_highlightcount"].clone(),
			roomuserid_lastnotificationread: db["userroomid_highlightcount"].clone(), //< NOTE: known bug from conduit
			roomsynctoken_shortstatehash: db["roomsynctoken_shortstatehash"].clone(),
			userroomid_joined: db["userroomid_joined"].clone(),
			services: Services {
				globals: args.depend::<globals::Service>("globals"),
				short: args.depend::<rooms::short::Service>("rooms::short"),
			},
		}
	}

	pub(super) fn reset_notification_counts(&self, user_id: &UserId, room_id: &RoomId) -> Result<()> {
		let mut userroom_id = user_id.as_bytes().to_vec();
		userroom_id.push(0xFF);
		userroom_id.extend_from_slice(room_id.as_bytes());
		let mut roomuser_id = room_id.as_bytes().to_vec();
		roomuser_id.push(0xFF);
		roomuser_id.extend_from_slice(user_id.as_bytes());

		self.userroomid_notificationcount
			.insert(&userroom_id, &0_u64.to_be_bytes())?;
		self.userroomid_highlightcount
			.insert(&userroom_id, &0_u64.to_be_bytes())?;

		self.roomuserid_lastnotificationread
			.insert(&roomuser_id, &self.services.globals.next_count()?.to_be_bytes())?;

		Ok(())
	}

	pub(super) fn notification_count(&self, user_id: &UserId, room_id: &RoomId) -> Result<u64> {
		let mut userroom_id = user_id.as_bytes().to_vec();
		userroom_id.push(0xFF);
		userroom_id.extend_from_slice(room_id.as_bytes());

		self.userroomid_notificationcount
			.get(&userroom_id)?
			.map_or(Ok(0), |bytes| {
				utils::u64_from_bytes(&bytes).map_err(|_| Error::bad_database("Invalid notification count in db."))
			})
	}

	pub(super) fn highlight_count(&self, user_id: &UserId, room_id: &RoomId) -> Result<u64> {
		let mut userroom_id = user_id.as_bytes().to_vec();
		userroom_id.push(0xFF);
		userroom_id.extend_from_slice(room_id.as_bytes());

		self.userroomid_highlightcount
			.get(&userroom_id)?
			.map_or(Ok(0), |bytes| {
				utils::u64_from_bytes(&bytes).map_err(|_| Error::bad_database("Invalid highlight count in db."))
			})
	}

	pub(super) fn last_notification_read(&self, user_id: &UserId, room_id: &RoomId) -> Result<u64> {
		let mut key = room_id.as_bytes().to_vec();
		key.push(0xFF);
		key.extend_from_slice(user_id.as_bytes());

		Ok(self
			.roomuserid_lastnotificationread
			.get(&key)?
			.map(|bytes| {
				utils::u64_from_bytes(&bytes)
					.map_err(|_| Error::bad_database("Count in roomuserid_lastprivatereadupdate is invalid."))
			})
			.transpose()?
			.unwrap_or(0))
	}

	pub(super) fn associate_token_shortstatehash(
		&self, room_id: &RoomId, token: u64, shortstatehash: u64,
	) -> Result<()> {
		let shortroomid = self
			.services
			.short
			.get_shortroomid(room_id)?
			.expect("room exists");

		let mut key = shortroomid.to_be_bytes().to_vec();
		key.extend_from_slice(&token.to_be_bytes());

		self.roomsynctoken_shortstatehash
			.insert(&key, &shortstatehash.to_be_bytes())
	}

	pub(super) fn get_token_shortstatehash(&self, room_id: &RoomId, token: u64) -> Result<Option<u64>> {
		let shortroomid = self
			.services
			.short
			.get_shortroomid(room_id)?
			.expect("room exists");

		let mut key = shortroomid.to_be_bytes().to_vec();
		key.extend_from_slice(&token.to_be_bytes());

		self.roomsynctoken_shortstatehash
			.get(&key)?
			.map(|bytes| {
				utils::u64_from_bytes(&bytes)
					.map_err(|_| Error::bad_database("Invalid shortstatehash in roomsynctoken_shortstatehash"))
			})
			.transpose()
	}

	pub(super) fn get_shared_rooms<'a>(
		&'a self, users: Vec<OwnedUserId>,
	) -> Result<Box<dyn Iterator<Item = Result<OwnedRoomId>> + 'a>> {
		let iterators = users.into_iter().map(move |user_id| {
			let mut prefix = user_id.as_bytes().to_vec();
			prefix.push(0xFF);

			self.userroomid_joined
				.scan_prefix(prefix)
				.map(|(key, _)| {
					let roomid_index = key
						.iter()
						.enumerate()
						.find(|(_, &b)| b == 0xFF)
						.ok_or_else(|| Error::bad_database("Invalid userroomid_joined in db."))?
						.0
						.saturating_add(1); // +1 because the room id starts AFTER the separator

					let room_id = key[roomid_index..].to_vec();

					Ok::<_, Error>(room_id)
				})
				.filter_map(Result::ok)
		});

		// We use the default compare function because keys are sorted correctly (not
		// reversed)
		Ok(Box::new(
			utils::common_elements(iterators, Ord::cmp)
				.expect("users is not empty")
				.map(|bytes| {
					RoomId::parse(
						utils::string_from_bytes(&bytes)
							.map_err(|_| Error::bad_database("Invalid RoomId bytes in userroomid_joined"))?,
					)
					.map_err(|_| Error::bad_database("Invalid RoomId in userroomid_joined."))
				}),
		))
	}
}
