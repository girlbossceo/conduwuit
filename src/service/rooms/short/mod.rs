use std::{mem::size_of_val, sync::Arc};

pub use conduit::pdu::{ShortEventId, ShortId, ShortRoomId};
use conduit::{err, implement, utils, Result};
use database::{Deserialized, Map};
use ruma::{events::StateEventType, EventId, RoomId};

use crate::{globals, Dep};

pub struct Service {
	db: Data,
	services: Services,
}

struct Data {
	eventid_shorteventid: Arc<Map>,
	shorteventid_eventid: Arc<Map>,
	statekey_shortstatekey: Arc<Map>,
	shortstatekey_statekey: Arc<Map>,
	roomid_shortroomid: Arc<Map>,
	statehash_shortstatehash: Arc<Map>,
}

struct Services {
	globals: Dep<globals::Service>,
}

pub type ShortStateHash = ShortId;
pub type ShortStateKey = ShortId;

impl crate::Service for Service {
	fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
		Ok(Arc::new(Self {
			db: Data {
				eventid_shorteventid: args.db["eventid_shorteventid"].clone(),
				shorteventid_eventid: args.db["shorteventid_eventid"].clone(),
				statekey_shortstatekey: args.db["statekey_shortstatekey"].clone(),
				shortstatekey_statekey: args.db["shortstatekey_statekey"].clone(),
				roomid_shortroomid: args.db["roomid_shortroomid"].clone(),
				statehash_shortstatehash: args.db["statehash_shortstatehash"].clone(),
			},
			services: Services {
				globals: args.depend::<globals::Service>("globals"),
			},
		}))
	}

	fn name(&self) -> &str { crate::service::make_name(std::module_path!()) }
}

#[implement(Service)]
pub async fn get_or_create_shorteventid(&self, event_id: &EventId) -> ShortEventId {
	const BUFSIZE: usize = size_of::<ShortEventId>();

	if let Ok(shorteventid) = self
		.db
		.eventid_shorteventid
		.get(event_id)
		.await
		.deserialized()
	{
		return shorteventid;
	}

	let shorteventid = self.services.globals.next_count().unwrap();
	debug_assert!(size_of_val(&shorteventid) == BUFSIZE, "buffer requirement changed");

	self.db
		.eventid_shorteventid
		.raw_aput::<BUFSIZE, _, _>(event_id, shorteventid);

	self.db
		.shorteventid_eventid
		.aput_raw::<BUFSIZE, _, _>(shorteventid, event_id);

	shorteventid
}

#[implement(Service)]
pub async fn multi_get_or_create_shorteventid(&self, event_ids: &[&EventId]) -> Vec<ShortEventId> {
	self.db
		.eventid_shorteventid
		.get_batch_blocking(event_ids.iter())
		.into_iter()
		.enumerate()
		.map(|(i, result)| match result {
			Ok(ref short) => utils::u64_from_u8(short),
			Err(_) => {
				const BUFSIZE: usize = size_of::<ShortEventId>();

				let short = self.services.globals.next_count().unwrap();
				debug_assert!(size_of_val(&short) == BUFSIZE, "buffer requirement changed");

				self.db
					.eventid_shorteventid
					.raw_aput::<BUFSIZE, _, _>(event_ids[i], short);
				self.db
					.shorteventid_eventid
					.aput_raw::<BUFSIZE, _, _>(short, event_ids[i]);

				short
			},
		})
		.collect()
}

#[implement(Service)]
pub async fn get_shortstatekey(&self, event_type: &StateEventType, state_key: &str) -> Result<ShortStateKey> {
	let key = (event_type, state_key);
	self.db
		.statekey_shortstatekey
		.qry(&key)
		.await
		.deserialized()
}

#[implement(Service)]
pub async fn get_or_create_shortstatekey(&self, event_type: &StateEventType, state_key: &str) -> ShortStateKey {
	const BUFSIZE: usize = size_of::<ShortStateKey>();

	let key = (event_type, state_key);
	if let Ok(shortstatekey) = self
		.db
		.statekey_shortstatekey
		.qry(&key)
		.await
		.deserialized()
	{
		return shortstatekey;
	}

	let shortstatekey = self.services.globals.next_count().unwrap();
	debug_assert!(size_of_val(&shortstatekey) == BUFSIZE, "buffer requirement changed");

	self.db
		.statekey_shortstatekey
		.put_aput::<BUFSIZE, _, _>(key, shortstatekey);

	self.db
		.shortstatekey_statekey
		.aput_put::<BUFSIZE, _, _>(shortstatekey, key);

	shortstatekey
}

#[implement(Service)]
pub async fn get_eventid_from_short(&self, shorteventid: ShortEventId) -> Result<Arc<EventId>> {
	const BUFSIZE: usize = size_of::<ShortEventId>();

	self.db
		.shorteventid_eventid
		.aqry::<BUFSIZE, _>(&shorteventid)
		.await
		.deserialized()
		.map_err(|e| err!(Database("Failed to find EventId from short {shorteventid:?}: {e:?}")))
}

#[implement(Service)]
pub async fn multi_get_eventid_from_short(&self, shorteventid: &[ShortEventId]) -> Vec<Result<Arc<EventId>>> {
	const BUFSIZE: usize = size_of::<ShortEventId>();

	let keys: Vec<[u8; BUFSIZE]> = shorteventid
		.iter()
		.map(|short| short.to_be_bytes())
		.collect();

	self.db
		.shorteventid_eventid
		.get_batch_blocking(keys.iter())
		.into_iter()
		.map(Deserialized::deserialized)
		.collect()
}

#[implement(Service)]
pub async fn get_statekey_from_short(&self, shortstatekey: ShortStateKey) -> Result<(StateEventType, String)> {
	const BUFSIZE: usize = size_of::<ShortStateKey>();

	self.db
		.shortstatekey_statekey
		.aqry::<BUFSIZE, _>(&shortstatekey)
		.await
		.deserialized()
		.map_err(|e| {
			err!(Database(
				"Failed to find (StateEventType, state_key) from short {shortstatekey:?}: {e:?}"
			))
		})
}

/// Returns (shortstatehash, already_existed)
#[implement(Service)]
pub async fn get_or_create_shortstatehash(&self, state_hash: &[u8]) -> (ShortStateHash, bool) {
	const BUFSIZE: usize = size_of::<ShortStateHash>();

	if let Ok(shortstatehash) = self
		.db
		.statehash_shortstatehash
		.get(state_hash)
		.await
		.deserialized()
	{
		return (shortstatehash, true);
	}

	let shortstatehash = self.services.globals.next_count().unwrap();
	debug_assert!(size_of_val(&shortstatehash) == BUFSIZE, "buffer requirement changed");

	self.db
		.statehash_shortstatehash
		.raw_aput::<BUFSIZE, _, _>(state_hash, shortstatehash);

	(shortstatehash, false)
}

#[implement(Service)]
pub async fn get_shortroomid(&self, room_id: &RoomId) -> Result<ShortRoomId> {
	self.db.roomid_shortroomid.get(room_id).await.deserialized()
}

#[implement(Service)]
pub async fn get_or_create_shortroomid(&self, room_id: &RoomId) -> ShortRoomId {
	self.db
		.roomid_shortroomid
		.get(room_id)
		.await
		.deserialized()
		.unwrap_or_else(|_| {
			const BUFSIZE: usize = size_of::<ShortRoomId>();

			let short = self.services.globals.next_count().unwrap();
			debug_assert!(size_of_val(&short) == BUFSIZE, "buffer requirement changed");

			self.db
				.roomid_shortroomid
				.raw_aput::<BUFSIZE, _, _>(room_id, short);

			short
		})
}
