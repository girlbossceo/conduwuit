use std::sync::Arc;

use conduit::Result;
use database::{Deserialized, Map};
use futures::{Stream, StreamExt};
use ruma::{RoomId, UserId};

use crate::{globals, rooms, Dep};

pub(super) struct Data {
	userroomid_notificationcount: Arc<Map>,
	userroomid_highlightcount: Arc<Map>,
	roomuserid_lastnotificationread: Arc<Map>,
	roomsynctoken_shortstatehash: Arc<Map>,
	services: Services,
}

struct Services {
	globals: Dep<globals::Service>,
	short: Dep<rooms::short::Service>,
	state_cache: Dep<rooms::state_cache::Service>,
}

impl Data {
	pub(super) fn new(args: &crate::Args<'_>) -> Self {
		let db = &args.db;
		Self {
			userroomid_notificationcount: db["userroomid_notificationcount"].clone(),
			userroomid_highlightcount: db["userroomid_highlightcount"].clone(),
			roomuserid_lastnotificationread: db["userroomid_highlightcount"].clone(), //< NOTE: known bug from conduit
			roomsynctoken_shortstatehash: db["roomsynctoken_shortstatehash"].clone(),
			services: Services {
				globals: args.depend::<globals::Service>("globals"),
				short: args.depend::<rooms::short::Service>("rooms::short"),
				state_cache: args.depend::<rooms::state_cache::Service>("rooms::state_cache"),
			},
		}
	}

	pub(super) fn reset_notification_counts(&self, user_id: &UserId, room_id: &RoomId) {
		let userroom_id = (user_id, room_id);
		self.userroomid_highlightcount.put(userroom_id, 0_u64);
		self.userroomid_notificationcount.put(userroom_id, 0_u64);

		let roomuser_id = (room_id, user_id);
		let count = self.services.globals.next_count().unwrap();
		self.roomuserid_lastnotificationread.put(roomuser_id, count);
	}

	pub(super) async fn notification_count(&self, user_id: &UserId, room_id: &RoomId) -> u64 {
		let key = (user_id, room_id);
		self.userroomid_notificationcount
			.qry(&key)
			.await
			.deserialized()
			.unwrap_or(0)
	}

	pub(super) async fn highlight_count(&self, user_id: &UserId, room_id: &RoomId) -> u64 {
		let key = (user_id, room_id);
		self.userroomid_highlightcount
			.qry(&key)
			.await
			.deserialized()
			.unwrap_or(0)
	}

	pub(super) async fn last_notification_read(&self, user_id: &UserId, room_id: &RoomId) -> u64 {
		let key = (room_id, user_id);
		self.roomuserid_lastnotificationread
			.qry(&key)
			.await
			.deserialized()
			.unwrap_or(0)
	}

	pub(super) async fn associate_token_shortstatehash(&self, room_id: &RoomId, token: u64, shortstatehash: u64) {
		let shortroomid = self
			.services
			.short
			.get_shortroomid(room_id)
			.await
			.expect("room exists");

		let key: &[u64] = &[shortroomid, token];
		self.roomsynctoken_shortstatehash.put(key, shortstatehash);
	}

	pub(super) async fn get_token_shortstatehash(&self, room_id: &RoomId, token: u64) -> Result<u64> {
		let shortroomid = self.services.short.get_shortroomid(room_id).await?;

		let key: &[u64] = &[shortroomid, token];
		self.roomsynctoken_shortstatehash
			.qry(key)
			.await
			.deserialized()
	}

	//TODO: optimize; replace point-queries with dual iteration
	pub(super) fn get_shared_rooms<'a>(
		&'a self, user_a: &'a UserId, user_b: &'a UserId,
	) -> impl Stream<Item = &RoomId> + Send + 'a {
		self.services
			.state_cache
			.rooms_joined(user_a)
			.filter(|room_id| self.services.state_cache.is_joined(user_b, room_id))
	}
}
