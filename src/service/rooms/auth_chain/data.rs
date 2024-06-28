use std::{mem::size_of, sync::Arc};

use conduit::{utils, Result};
use database::{Database, Map};

pub(super) struct Data {
	shorteventid_authchain: Arc<Map>,
	db: Arc<Database>,
}

impl Data {
	pub(super) fn new(db: &Arc<Database>) -> Self {
		Self {
			shorteventid_authchain: db["shorteventid_authchain"].clone(),
			db: db.clone(),
		}
	}

	pub(super) fn get_cached_eventid_authchain(&self, key: &[u64]) -> Result<Option<Arc<[u64]>>> {
		// Check RAM cache
		if let Some(result) = self.db.auth_chain_cache.lock().unwrap().get_mut(key) {
			return Ok(Some(Arc::clone(result)));
		}

		// We only save auth chains for single events in the db
		if key.len() == 1 {
			// Check DB cache
			let chain = self
				.shorteventid_authchain
				.get(&key[0].to_be_bytes())?
				.map(|chain| {
					chain
						.chunks_exact(size_of::<u64>())
						.map(|chunk| utils::u64_from_bytes(chunk).expect("byte length is correct"))
						.collect::<Arc<[u64]>>()
				});

			if let Some(chain) = chain {
				// Cache in RAM
				self.db
					.auth_chain_cache
					.lock()
					.unwrap()
					.insert(vec![key[0]], Arc::clone(&chain));

				return Ok(Some(chain));
			}
		}

		Ok(None)
	}

	pub(super) fn cache_auth_chain(&self, key: Vec<u64>, auth_chain: Arc<[u64]>) -> Result<()> {
		// Only persist single events in db
		if key.len() == 1 {
			self.shorteventid_authchain.insert(
				&key[0].to_be_bytes(),
				&auth_chain
					.iter()
					.flat_map(|s| s.to_be_bytes().to_vec())
					.collect::<Vec<u8>>(),
			)?;
		}

		// Cache in RAM
		self.db
			.auth_chain_cache
			.lock()
			.unwrap()
			.insert(key, auth_chain);

		Ok(())
	}
}
