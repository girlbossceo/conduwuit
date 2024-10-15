use std::sync::Arc;

use conduit::{implement, Result};
use database::{Deserialized, Map};
use futures::{pin_mut, Stream, StreamExt};
use ruma::{RoomId, UserId};

use crate::{globals, rooms, Dep};

pub struct Service {
	db: Data,
	services: Services,
}

struct Data {
	userroomid_notificationcount: Arc<Map>,
	userroomid_highlightcount: Arc<Map>,
	roomuserid_lastnotificationread: Arc<Map>,
	roomsynctoken_shortstatehash: Arc<Map>,
}

struct Services {
	globals: Dep<globals::Service>,
	short: Dep<rooms::short::Service>,
	state_cache: Dep<rooms::state_cache::Service>,
}

impl crate::Service for Service {
	fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
		Ok(Arc::new(Self {
			db: Data {
			userroomid_notificationcount: args.db["userroomid_notificationcount"].clone(),
			userroomid_highlightcount: args.db["userroomid_highlightcount"].clone(),
			roomuserid_lastnotificationread: args.db["userroomid_highlightcount"].clone(), //< NOTE: known bug from conduit
			roomsynctoken_shortstatehash: args.db["roomsynctoken_shortstatehash"].clone(),
		},

			services: Services {
				globals: args.depend::<globals::Service>("globals"),
				short: args.depend::<rooms::short::Service>("rooms::short"),
				state_cache: args.depend::<rooms::state_cache::Service>("rooms::state_cache"),
			},
		}))
	}

	fn name(&self) -> &str { crate::service::make_name(std::module_path!()) }
}

#[implement(Service)]
pub fn reset_notification_counts(&self, user_id: &UserId, room_id: &RoomId) {
	let userroom_id = (user_id, room_id);
	self.db.userroomid_highlightcount.put(userroom_id, 0_u64);
	self.db.userroomid_notificationcount.put(userroom_id, 0_u64);

	let roomuser_id = (room_id, user_id);
	let count = self.services.globals.next_count().unwrap();
	self.db
		.roomuserid_lastnotificationread
		.put(roomuser_id, count);
}

#[implement(Service)]
pub async fn notification_count(&self, user_id: &UserId, room_id: &RoomId) -> u64 {
	let key = (user_id, room_id);
	self.db
		.userroomid_notificationcount
		.qry(&key)
		.await
		.deserialized()
		.unwrap_or(0)
}

#[implement(Service)]
pub async fn highlight_count(&self, user_id: &UserId, room_id: &RoomId) -> u64 {
	let key = (user_id, room_id);
	self.db
		.userroomid_highlightcount
		.qry(&key)
		.await
		.deserialized()
		.unwrap_or(0)
}

#[implement(Service)]
pub async fn last_notification_read(&self, user_id: &UserId, room_id: &RoomId) -> u64 {
	let key = (room_id, user_id);
	self.db
		.roomuserid_lastnotificationread
		.qry(&key)
		.await
		.deserialized()
		.unwrap_or(0)
}

#[implement(Service)]
pub async fn associate_token_shortstatehash(&self, room_id: &RoomId, token: u64, shortstatehash: u64) {
	let shortroomid = self
		.services
		.short
		.get_shortroomid(room_id)
		.await
		.expect("room exists");

	let key: &[u64] = &[shortroomid, token];
	self.db
		.roomsynctoken_shortstatehash
		.put(key, shortstatehash);
}

#[implement(Service)]
pub async fn get_token_shortstatehash(&self, room_id: &RoomId, token: u64) -> Result<u64> {
	let shortroomid = self.services.short.get_shortroomid(room_id).await?;

	let key: &[u64] = &[shortroomid, token];
	self.db
		.roomsynctoken_shortstatehash
		.qry(key)
		.await
		.deserialized()
}

#[implement(Service)]
pub async fn has_shared_rooms<'a>(&'a self, user_a: &'a UserId, user_b: &'a UserId) -> bool {
	let get_shared_rooms = self.get_shared_rooms(user_a, user_b);

	pin_mut!(get_shared_rooms);
	get_shared_rooms.next().await.is_some()
}

//TODO: optimize; replace point-queries with dual iteration
#[implement(Service)]
pub fn get_shared_rooms<'a>(
	&'a self, user_a: &'a UserId, user_b: &'a UserId,
) -> impl Stream<Item = &RoomId> + Send + 'a {
	self.services
		.state_cache
		.rooms_joined(user_a)
		.filter(|room_id| self.services.state_cache.is_joined(user_b, room_id))
}
