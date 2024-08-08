use std::sync::Arc;

use conduit::utils::{set, stream::TryIgnore, IterStream, ReadyExt};
use database::Map;
use futures::StreamExt;
use ruma::RoomId;

use crate::{rooms, Dep};

pub(super) struct Data {
	tokenids: Arc<Map>,
	services: Services,
}

struct Services {
	short: Dep<rooms::short::Service>,
}

impl Data {
	pub(super) fn new(args: &crate::Args<'_>) -> Self {
		let db = &args.db;
		Self {
			tokenids: db["tokenids"].clone(),
			services: Services {
				short: args.depend::<rooms::short::Service>("rooms::short"),
			},
		}
	}

	pub(super) fn index_pdu(&self, shortroomid: u64, pdu_id: &[u8], message_body: &str) {
		let batch = tokenize(message_body)
			.map(|word| {
				let mut key = shortroomid.to_be_bytes().to_vec();
				key.extend_from_slice(word.as_bytes());
				key.push(0xFF);
				key.extend_from_slice(pdu_id); // TODO: currently we save the room id a second time here
				(key, Vec::<u8>::new())
			})
			.collect::<Vec<_>>();

		self.tokenids.insert_batch(batch.iter());
	}

	pub(super) fn deindex_pdu(&self, shortroomid: u64, pdu_id: &[u8], message_body: &str) {
		let batch = tokenize(message_body).map(|word| {
			let mut key = shortroomid.to_be_bytes().to_vec();
			key.extend_from_slice(word.as_bytes());
			key.push(0xFF);
			key.extend_from_slice(pdu_id); // TODO: currently we save the room id a second time here
			key
		});

		for token in batch {
			self.tokenids.remove(&token);
		}
	}

	pub(super) async fn search_pdus(
		&self, room_id: &RoomId, search_string: &str,
	) -> Option<(Vec<Vec<u8>>, Vec<String>)> {
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

				self.tokenids
				.rev_raw_keys_from(&last_possible_id) // Newest pdus first
				.ignore_err()
				.ready_take_while(move |key| key.starts_with(&prefix2))
				.map(move |key| key[prefix3.len()..].to_vec())
				.collect::<Vec<_>>()
			})
			.collect()
			.await;

		Some((
			set::intersection(bufs.iter().map(|buf| buf.iter()))
				.cloned()
				.collect(),
			words,
		))
	}
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
