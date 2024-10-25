use std::sync::Arc;

use conduit::{
	implement,
	utils::{set, stream::TryIgnore, IterStream, ReadyExt},
	Result,
};
use database::Map;
use futures::StreamExt;
use ruma::RoomId;

use crate::{rooms, Dep};

pub struct Service {
	db: Data,
	services: Services,
}

struct Data {
	tokenids: Arc<Map>,
}

struct Services {
	short: Dep<rooms::short::Service>,
}

impl crate::Service for Service {
	fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
		Ok(Arc::new(Self {
			db: Data {
				tokenids: args.db["tokenids"].clone(),
			},
			services: Services {
				short: args.depend::<rooms::short::Service>("rooms::short"),
			},
		}))
	}

	fn name(&self) -> &str { crate::service::make_name(std::module_path!()) }
}

#[implement(Service)]
pub fn index_pdu(&self, shortroomid: u64, pdu_id: &[u8], message_body: &str) {
	let batch = tokenize(message_body)
		.map(|word| {
			let mut key = shortroomid.to_be_bytes().to_vec();
			key.extend_from_slice(word.as_bytes());
			key.push(0xFF);
			key.extend_from_slice(pdu_id); // TODO: currently we save the room id a second time here
			(key, Vec::<u8>::new())
		})
		.collect::<Vec<_>>();

	self.db.tokenids.insert_batch(batch.iter());
}

#[implement(Service)]
pub fn deindex_pdu(&self, shortroomid: u64, pdu_id: &[u8], message_body: &str) {
	let batch = tokenize(message_body).map(|word| {
		let mut key = shortroomid.to_be_bytes().to_vec();
		key.extend_from_slice(word.as_bytes());
		key.push(0xFF);
		key.extend_from_slice(pdu_id); // TODO: currently we save the room id a second time here
		key
	});

	for token in batch {
		self.db.tokenids.remove(&token);
	}
}

#[implement(Service)]
pub async fn search_pdus(&self, room_id: &RoomId, search_string: &str) -> Option<(Vec<Vec<u8>>, Vec<String>)> {
	let prefix = self
		.services
		.short
		.get_shortroomid(room_id)
		.await
		.ok()?
		.to_be_bytes()
		.to_vec();

	let words: Vec<_> = tokenize(search_string).collect();

	let bufs: Vec<_> = words
		.clone()
		.into_iter()
		.stream()
		.then(move |word| {
			let mut prefix2 = prefix.clone();
			prefix2.extend_from_slice(word.as_bytes());
			prefix2.push(0xFF);
			let prefix3 = prefix2.clone();

			let mut last_possible_id = prefix2.clone();
			last_possible_id.extend_from_slice(&u64::MAX.to_be_bytes());

			self.db.tokenids
				.rev_raw_keys_from(&last_possible_id) // Newest pdus first
				.ignore_err()
				.ready_take_while(move |key| key.starts_with(&prefix2))
				.map(move |key| key[prefix3.len()..].to_vec())
				.collect::<Vec<_>>()
		})
		.collect()
		.await;

	let bufs = bufs.iter().map(|buf| buf.iter());

	let results = set::intersection(bufs).cloned().collect();

	Some((results, words))
}

/// Splits a string into tokens used as keys in the search inverted index
///
/// This may be used to tokenize both message bodies (for indexing) or search
/// queries (for querying).
fn tokenize(body: &str) -> impl Iterator<Item = String> + Send + '_ {
	body.split_terminator(|c: char| !c.is_alphanumeric())
		.filter(|s| !s.is_empty())
		.filter(|word| word.len() <= 50)
		.map(str::to_lowercase)
}
