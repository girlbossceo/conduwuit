mod data;

use std::sync::Arc;

use conduit::Result;
use ruma::{events::StateEventType, EventId, RoomId};

use self::data::Data;

pub struct Service {
	db: Data,
}

impl crate::Service for Service {
	fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
		Ok(Arc::new(Self {
			db: Data::new(&args),
		}))
	}

	fn name(&self) -> &str { crate::service::make_name(std::module_path!()) }
}

impl Service {
	pub async fn get_or_create_shorteventid(&self, event_id: &EventId) -> u64 {
		self.db.get_or_create_shorteventid(event_id).await
	}

	pub async fn multi_get_or_create_shorteventid(&self, event_ids: &[&EventId]) -> Vec<u64> {
		self.db.multi_get_or_create_shorteventid(event_ids).await
	}

	pub async fn get_shortstatekey(&self, event_type: &StateEventType, state_key: &str) -> Result<u64> {
		self.db.get_shortstatekey(event_type, state_key).await
	}

	pub async fn get_or_create_shortstatekey(&self, event_type: &StateEventType, state_key: &str) -> u64 {
		self.db
			.get_or_create_shortstatekey(event_type, state_key)
			.await
	}

	pub async fn get_eventid_from_short(&self, shorteventid: u64) -> Result<Arc<EventId>> {
		self.db.get_eventid_from_short(shorteventid).await
	}

	pub async fn get_statekey_from_short(&self, shortstatekey: u64) -> Result<(StateEventType, String)> {
		self.db.get_statekey_from_short(shortstatekey).await
	}

	/// Returns (shortstatehash, already_existed)
	pub async fn get_or_create_shortstatehash(&self, state_hash: &[u8]) -> (u64, bool) {
		self.db.get_or_create_shortstatehash(state_hash).await
	}

	pub async fn get_shortroomid(&self, room_id: &RoomId) -> Result<u64> { self.db.get_shortroomid(room_id).await }

	pub async fn get_or_create_shortroomid(&self, room_id: &RoomId) -> u64 {
		self.db.get_or_create_shortroomid(room_id).await
	}
}
