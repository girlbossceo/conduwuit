use std::sync::Arc;

use conduit::{utils, Result};
use database::Map;
use ruma::RoomId;

use crate::{rooms, Dep};

type SearchPdusResult<'a> = Result<Option<(Box<dyn Iterator<Item = Vec<u8>> + 'a>, Vec<String>)>>;

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

	pub(super) fn index_pdu(&self, shortroomid: u64, pdu_id: &[u8], message_body: &str) -> Result<()> {
		let batch = tokenize(message_body)
			.map(|word| {
				let mut key = shortroomid.to_be_bytes().to_vec();
				key.extend_from_slice(word.as_bytes());
				key.push(0xFF);
				key.extend_from_slice(pdu_id); // TODO: currently we save the room id a second time here
				(key, Vec::<u8>::new())
			})
			.collect::<Vec<_>>();

		self.tokenids
			.insert_batch(batch.iter().map(database::KeyVal::from))
	}

	pub(super) fn deindex_pdu(&self, shortroomid: u64, pdu_id: &[u8], message_body: &str) -> Result<()> {
		let batch = tokenize(message_body).map(|word| {
			let mut key = shortroomid.to_be_bytes().to_vec();
			key.extend_from_slice(word.as_bytes());
			key.push(0xFF);
			key.extend_from_slice(pdu_id); // TODO: currently we save the room id a second time here
			key
		});

		for token in batch {
			self.tokenids.remove(&token)?;
		}

		Ok(())
	}

	pub(super) fn search_pdus<'a>(&'a self, room_id: &RoomId, search_string: &str) -> SearchPdusResult<'a> {
		let prefix = self
			.services
			.short
			.get_shortroomid(room_id)?
			.expect("room exists")
			.to_be_bytes()
			.to_vec();

		let words: Vec<_> = tokenize(search_string).collect();

		let iterators = words.clone().into_iter().map(move |word| {
			let mut prefix2 = prefix.clone();
			prefix2.extend_from_slice(word.as_bytes());
			prefix2.push(0xFF);
			let prefix3 = prefix2.clone();

			let mut last_possible_id = prefix2.clone();
			last_possible_id.extend_from_slice(&u64::MAX.to_be_bytes());

			self.tokenids
				.iter_from(&last_possible_id, true) // Newest pdus first
				.take_while(move |(k, _)| k.starts_with(&prefix2))
				.map(move |(key, _)| key[prefix3.len()..].to_vec())
		});

		let Some(common_elements) = utils::common_elements(iterators, |a, b| {
			// We compare b with a because we reversed the iterator earlier
			b.cmp(a)
		}) else {
			return Ok(None);
		};

		Ok(Some((Box::new(common_elements), words)))
	}
}

/// Splits a string into tokens used as keys in the search inverted index
///
/// This may be used to tokenize both message bodies (for indexing) or search
/// queries (for querying).
fn tokenize(body: &str) -> impl Iterator<Item = String> + '_ {
	body.split_terminator(|c: char| !c.is_alphanumeric())
		.filter(|s| !s.is_empty())
		.filter(|word| word.len() <= 50)
		.map(str::to_lowercase)
}
