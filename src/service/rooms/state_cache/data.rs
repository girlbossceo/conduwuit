use std::{
	collections::HashMap,
	sync::{Arc, RwLock},
};

use conduit::{utils::stream::TryIgnore, Result};
use database::{serialize_to_vec, Deserialized, Interfix, Json, Map};
use futures::{Stream, StreamExt};
use ruma::{
	events::{AnyStrippedStateEvent, AnySyncStateEvent},
	serde::Raw,
	OwnedRoomId, RoomId, UserId,
};

use crate::{globals, Dep};

type AppServiceInRoomCache = RwLock<HashMap<OwnedRoomId, HashMap<String, bool>>>;
type StrippedStateEventItem = (OwnedRoomId, Vec<Raw<AnyStrippedStateEvent>>);
type SyncStateEventItem = (OwnedRoomId, Vec<Raw<AnySyncStateEvent>>);

pub(super) struct Data {
	pub(super) appservice_in_room_cache: AppServiceInRoomCache,
	pub(super) roomid_invitedcount: Arc<Map>,
	pub(super) roomid_inviteviaservers: Arc<Map>,
	pub(super) roomid_joinedcount: Arc<Map>,
	pub(super) roomserverids: Arc<Map>,
	pub(super) roomuserid_invitecount: Arc<Map>,
	pub(super) roomuserid_joined: Arc<Map>,
	pub(super) roomuserid_leftcount: Arc<Map>,
	pub(super) roomuseroncejoinedids: Arc<Map>,
	pub(super) serverroomids: Arc<Map>,
	pub(super) userroomid_invitestate: Arc<Map>,
	pub(super) userroomid_joined: Arc<Map>,
	pub(super) userroomid_leftstate: Arc<Map>,
	services: Services,
}

struct Services {
	globals: Dep<globals::Service>,
}

impl Data {
	pub(super) fn new(args: &crate::Args<'_>) -> Self {
		let db = &args.db;
		Self {
			appservice_in_room_cache: RwLock::new(HashMap::new()),
			roomid_invitedcount: db["roomid_invitedcount"].clone(),
			roomid_inviteviaservers: db["roomid_inviteviaservers"].clone(),
			roomid_joinedcount: db["roomid_joinedcount"].clone(),
			roomserverids: db["roomserverids"].clone(),
			roomuserid_invitecount: db["roomuserid_invitecount"].clone(),
			roomuserid_joined: db["roomuserid_joined"].clone(),
			roomuserid_leftcount: db["roomuserid_leftcount"].clone(),
			roomuseroncejoinedids: db["roomuseroncejoinedids"].clone(),
			serverroomids: db["serverroomids"].clone(),
			userroomid_invitestate: db["userroomid_invitestate"].clone(),
			userroomid_joined: db["userroomid_joined"].clone(),
			userroomid_leftstate: db["userroomid_leftstate"].clone(),
			services: Services {
				globals: args.depend::<globals::Service>("globals"),
			},
		}
	}

	pub(super) fn mark_as_once_joined(&self, user_id: &UserId, room_id: &RoomId) {
		let key = (user_id, room_id);

		self.roomuseroncejoinedids.put_raw(key, []);
	}

	pub(super) fn mark_as_joined(&self, user_id: &UserId, room_id: &RoomId) {
		let userroom_id = (user_id, room_id);
		let userroom_id = serialize_to_vec(userroom_id).expect("failed to serialize userroom_id");

		let roomuser_id = (room_id, user_id);
		let roomuser_id = serialize_to_vec(roomuser_id).expect("failed to serialize roomuser_id");

		self.userroomid_joined.insert(&userroom_id, []);
		self.roomuserid_joined.insert(&roomuser_id, []);

		self.userroomid_invitestate.remove(&userroom_id);
		self.roomuserid_invitecount.remove(&roomuser_id);

		self.userroomid_leftstate.remove(&userroom_id);
		self.roomuserid_leftcount.remove(&roomuser_id);

		self.roomid_inviteviaservers.remove(room_id);
	}

	pub(super) fn mark_as_left(&self, user_id: &UserId, room_id: &RoomId) {
		let userroom_id = (user_id, room_id);
		let userroom_id = serialize_to_vec(userroom_id).expect("failed to serialize userroom_id");

		let roomuser_id = (room_id, user_id);
		let roomuser_id = serialize_to_vec(roomuser_id).expect("failed to serialize roomuser_id");

		// (timo) TODO
		let leftstate = Vec::<Raw<AnySyncStateEvent>>::new();
		let count = self.services.globals.next_count().unwrap();

		self.userroomid_leftstate
			.raw_put(&userroom_id, Json(leftstate));
		self.roomuserid_leftcount.raw_put(&roomuser_id, count);

		self.userroomid_joined.remove(&userroom_id);
		self.roomuserid_joined.remove(&roomuser_id);

		self.userroomid_invitestate.remove(&userroom_id);
		self.roomuserid_invitecount.remove(&roomuser_id);

		self.roomid_inviteviaservers.remove(room_id);
	}

	/// Makes a user forget a room.
	#[tracing::instrument(skip(self), level = "debug")]
	pub(super) fn forget(&self, room_id: &RoomId, user_id: &UserId) {
		let userroom_id = (user_id, room_id);
		let roomuser_id = (room_id, user_id);

		self.userroomid_leftstate.del(userroom_id);
		self.roomuserid_leftcount.del(roomuser_id);
	}

	/// Returns an iterator over all rooms a user was invited to.
	#[inline]
	pub(super) fn rooms_invited<'a>(
		&'a self, user_id: &'a UserId,
	) -> impl Stream<Item = StrippedStateEventItem> + Send + 'a {
		type Key<'a> = (&'a UserId, &'a RoomId);
		type KeyVal<'a> = (Key<'a>, Raw<Vec<AnyStrippedStateEvent>>);

		let prefix = (user_id, Interfix);
		self.userroomid_invitestate
			.stream_prefix(&prefix)
			.ignore_err()
			.map(|((_, room_id), state): KeyVal<'_>| (room_id.to_owned(), state))
			.map(|(room_id, state)| Ok((room_id, state.deserialize_as()?)))
			.ignore_err()
	}

	/// Returns an iterator over all rooms a user left.
	#[inline]
	pub(super) fn rooms_left<'a>(&'a self, user_id: &'a UserId) -> impl Stream<Item = SyncStateEventItem> + Send + 'a {
		type Key<'a> = (&'a UserId, &'a RoomId);
		type KeyVal<'a> = (Key<'a>, Raw<Vec<Raw<AnySyncStateEvent>>>);

		let prefix = (user_id, Interfix);
		self.userroomid_leftstate
			.stream_prefix(&prefix)
			.ignore_err()
			.map(|((_, room_id), state): KeyVal<'_>| (room_id.to_owned(), state))
			.map(|(room_id, state)| Ok((room_id, state.deserialize_as()?)))
			.ignore_err()
	}

	#[tracing::instrument(skip(self), level = "debug")]
	pub(super) async fn invite_state(
		&self, user_id: &UserId, room_id: &RoomId,
	) -> Result<Vec<Raw<AnyStrippedStateEvent>>> {
		let key = (user_id, room_id);
		self.userroomid_invitestate
			.qry(&key)
			.await
			.deserialized()
			.and_then(|val: Raw<Vec<AnyStrippedStateEvent>>| val.deserialize_as().map_err(Into::into))
	}

	#[tracing::instrument(skip(self), level = "debug")]
	pub(super) async fn left_state(
		&self, user_id: &UserId, room_id: &RoomId,
	) -> Result<Vec<Raw<AnyStrippedStateEvent>>> {
		let key = (user_id, room_id);
		self.userroomid_leftstate
			.qry(&key)
			.await
			.deserialized()
			.and_then(|val: Raw<Vec<AnyStrippedStateEvent>>| val.deserialize_as().map_err(Into::into))
	}
}
