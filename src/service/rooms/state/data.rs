use std::sync::Arc;

use conduit::{
	utils::{stream::TryIgnore, ReadyExt},
	Result,
};
use database::{Database, Deserialized, Interfix, Map};
use ruma::{OwnedEventId, RoomId};

use super::RoomMutexGuard;

pub(super) struct Data {
	shorteventid_shortstatehash: Arc<Map>,
	roomid_shortstatehash: Arc<Map>,
	pub(super) roomid_pduleaves: Arc<Map>,
}

impl Data {
	pub(super) fn new(db: &Arc<Database>) -> Self {
		Self {
			shorteventid_shortstatehash: db["shorteventid_shortstatehash"].clone(),
			roomid_shortstatehash: db["roomid_shortstatehash"].clone(),
			roomid_pduleaves: db["roomid_pduleaves"].clone(),
		}
	}

	pub(super) async fn get_room_shortstatehash(&self, room_id: &RoomId) -> Result<u64> {
		self.roomid_shortstatehash.get(room_id).await.deserialized()
	}

	#[inline]
	pub(super) fn set_room_state(
		&self,
		room_id: &RoomId,
		new_shortstatehash: u64,
		_mutex_lock: &RoomMutexGuard, // Take mutex guard to make sure users get the room state mutex
	) {
		self.roomid_shortstatehash
			.insert(room_id.as_bytes(), &new_shortstatehash.to_be_bytes());
	}

	pub(super) fn set_event_state(&self, shorteventid: u64, shortstatehash: u64) {
		self.shorteventid_shortstatehash
			.insert(&shorteventid.to_be_bytes(), &shortstatehash.to_be_bytes());
	}

	pub(super) async fn set_forward_extremities(
		&self,
		room_id: &RoomId,
		event_ids: Vec<OwnedEventId>,
		_mutex_lock: &RoomMutexGuard, // Take mutex guard to make sure users get the room state mutex
	) {
		let prefix = (room_id, Interfix);
		self.roomid_pduleaves
			.keys_raw_prefix(&prefix)
			.ignore_err()
			.ready_for_each(|key| self.roomid_pduleaves.remove(key))
			.await;

		let mut prefix = room_id.as_bytes().to_vec();
		prefix.push(0xFF);
		for event_id in event_ids {
			let mut key = prefix.clone();
			key.extend_from_slice(event_id.as_bytes());
			self.roomid_pduleaves.insert(&key, event_id.as_bytes());
		}
	}
}
