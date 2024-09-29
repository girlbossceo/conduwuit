use std::sync::Arc;

use conduit::{err, utils, Error, Result};
use database::{Deserialized, Map};
use ruma::{events::StateEventType, EventId, RoomId};

use crate::{globals, Dep};

pub(super) struct Data {
	eventid_shorteventid: Arc<Map>,
	shorteventid_eventid: Arc<Map>,
	statekey_shortstatekey: Arc<Map>,
	shortstatekey_statekey: Arc<Map>,
	roomid_shortroomid: Arc<Map>,
	statehash_shortstatehash: Arc<Map>,
	services: Services,
}

struct Services {
	globals: Dep<globals::Service>,
}

impl Data {
	pub(super) fn new(args: &crate::Args<'_>) -> Self {
		let db = &args.db;
		Self {
			eventid_shorteventid: db["eventid_shorteventid"].clone(),
			shorteventid_eventid: db["shorteventid_eventid"].clone(),
			statekey_shortstatekey: db["statekey_shortstatekey"].clone(),
			shortstatekey_statekey: db["shortstatekey_statekey"].clone(),
			roomid_shortroomid: db["roomid_shortroomid"].clone(),
			statehash_shortstatehash: db["statehash_shortstatehash"].clone(),
			services: Services {
				globals: args.depend::<globals::Service>("globals"),
			},
		}
	}

	pub(super) async fn get_or_create_shorteventid(&self, event_id: &EventId) -> u64 {
		if let Ok(shorteventid) = self.eventid_shorteventid.qry(event_id).await.deserialized() {
			return shorteventid;
		}

		let shorteventid = self.services.globals.next_count().unwrap();
		self.eventid_shorteventid
			.insert(event_id.as_bytes(), &shorteventid.to_be_bytes());
		self.shorteventid_eventid
			.insert(&shorteventid.to_be_bytes(), event_id.as_bytes());

		shorteventid
	}

	pub(super) async fn multi_get_or_create_shorteventid(&self, event_ids: &[&EventId]) -> Vec<u64> {
		let mut ret: Vec<u64> = Vec::with_capacity(event_ids.len());
		let keys = event_ids
			.iter()
			.map(|id| id.as_bytes())
			.collect::<Vec<&[u8]>>();

		for (i, short) in self
			.eventid_shorteventid
			.get_batch_blocking(keys.iter())
			.iter()
			.enumerate()
		{
			match short {
				Some(short) => ret.push(
					utils::u64_from_bytes(short)
						.map_err(|_| Error::bad_database("Invalid shorteventid in db."))
						.unwrap(),
				),
				None => {
					let short = self.services.globals.next_count().unwrap();
					self.eventid_shorteventid
						.insert(keys[i], &short.to_be_bytes());
					self.shorteventid_eventid
						.insert(&short.to_be_bytes(), keys[i]);

					debug_assert!(ret.len() == i, "position of result must match input");
					ret.push(short);
				},
			}
		}

		ret
	}

	pub(super) async fn get_shortstatekey(&self, event_type: &StateEventType, state_key: &str) -> Result<u64> {
		let key = (event_type, state_key);
		self.statekey_shortstatekey.qry(&key).await.deserialized()
	}

	pub(super) async fn get_or_create_shortstatekey(&self, event_type: &StateEventType, state_key: &str) -> u64 {
		let key = (event_type.to_string(), state_key);
		if let Ok(shortstatekey) = self.statekey_shortstatekey.qry(&key).await.deserialized() {
			return shortstatekey;
		}

		let mut key = event_type.to_string().as_bytes().to_vec();
		key.push(0xFF);
		key.extend_from_slice(state_key.as_bytes());

		let shortstatekey = self.services.globals.next_count().unwrap();
		self.statekey_shortstatekey
			.insert(&key, &shortstatekey.to_be_bytes());
		self.shortstatekey_statekey
			.insert(&shortstatekey.to_be_bytes(), &key);

		shortstatekey
	}

	pub(super) async fn get_eventid_from_short(&self, shorteventid: u64) -> Result<Arc<EventId>> {
		self.shorteventid_eventid
			.qry(&shorteventid)
			.await
			.deserialized()
			.map_err(|e| err!(Database("Failed to find EventId from short {shorteventid:?}: {e:?}")))
	}

	pub(super) async fn get_statekey_from_short(&self, shortstatekey: u64) -> Result<(StateEventType, String)> {
		self.shortstatekey_statekey
			.qry(&shortstatekey)
			.await
			.deserialized()
			.map_err(|e| {
				err!(Database(
					"Failed to find (StateEventType, state_key) from short {shortstatekey:?}: {e:?}"
				))
			})
	}

	/// Returns (shortstatehash, already_existed)
	pub(super) async fn get_or_create_shortstatehash(&self, state_hash: &[u8]) -> (u64, bool) {
		if let Ok(shortstatehash) = self
			.statehash_shortstatehash
			.qry(state_hash)
			.await
			.deserialized()
		{
			return (shortstatehash, true);
		}

		let shortstatehash = self.services.globals.next_count().unwrap();
		self.statehash_shortstatehash
			.insert(state_hash, &shortstatehash.to_be_bytes());

		(shortstatehash, false)
	}

	pub(super) async fn get_shortroomid(&self, room_id: &RoomId) -> Result<u64> {
		self.roomid_shortroomid.qry(room_id).await.deserialized()
	}

	pub(super) async fn get_or_create_shortroomid(&self, room_id: &RoomId) -> u64 {
		self.roomid_shortroomid
			.qry(room_id)
			.await
			.deserialized()
			.unwrap_or_else(|_| {
				let short = self.services.globals.next_count().unwrap();
				self.roomid_shortroomid
					.insert(room_id.as_bytes(), &short.to_be_bytes());
				short
			})
	}
}
