mod data;

use std::sync::Arc;

use conduit::{Result, Server};
use data::Data;
use database::Database;
use ruma::RoomId;

pub struct Service {
	db: Data,
}

impl Service {
	pub fn build(_server: &Arc<Server>, db: &Arc<Database>) -> Result<Self> {
		Ok(Self {
			db: Data::new(db),
		})
	}

	#[tracing::instrument(skip(self))]
	pub fn index_pdu(&self, shortroomid: u64, pdu_id: &[u8], message_body: &str) -> Result<()> {
		self.db.index_pdu(shortroomid, pdu_id, message_body)
	}

	#[tracing::instrument(skip(self))]
	pub fn deindex_pdu(&self, shortroomid: u64, pdu_id: &[u8], message_body: &str) -> Result<()> {
		self.db.deindex_pdu(shortroomid, pdu_id, message_body)
	}

	#[tracing::instrument(skip(self))]
	pub fn search_pdus<'a>(
		&'a self, room_id: &RoomId, search_string: &str,
	) -> Result<Option<(impl Iterator<Item = Vec<u8>> + 'a, Vec<String>)>> {
		self.db.search_pdus(room_id, search_string)
	}
}
