use std::{
	mem::size_of,
	sync::{Arc, Mutex},
};

use conduit::{utils, utils::math::usize_from_f64, Result};
use database::Map;
use lru_cache::LruCache;

pub(super) struct Data {
	shorteventid_authchain: Arc<Map>,
	pub(super) auth_chain_cache: Mutex<LruCache<Vec<u64>, Arc<[u64]>>>,
}

impl Data {
	pub(super) fn new(args: &crate::Args<'_>) -> Self {
		let db = &args.db;
		let config = &args.server.config;
		let cache_size = f64::from(config.auth_chain_cache_capacity);
		let cache_size = usize_from_f64(cache_size * config.cache_capacity_modifier).expect("valid cache size");
		Self {
			shorteventid_authchain: db["shorteventid_authchain"].clone(),
			auth_chain_cache: Mutex::new(LruCache::new(cache_size)),
		}
	}

	pub(super) async fn get_cached_eventid_authchain(&self, key: &[u64]) -> Result<Option<Arc<[u64]>>> {
		// Check RAM cache
		if let Some(result) = self.auth_chain_cache.lock().unwrap().get_mut(key) {
			return Ok(Some(Arc::clone(result)));
		}

		// We only save auth chains for single events in the db
		if key.len() == 1 {
			// Check DB cache
			let chain = self.shorteventid_authchain.qry(&key[0]).await.map(|chain| {
				chain
					.chunks_exact(size_of::<u64>())
					.map(utils::u64_from_u8)
					.collect::<Arc<[u64]>>()
			});

			if let Ok(chain) = chain {
				// Cache in RAM
				self.auth_chain_cache
					.lock()
					.expect("locked")
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
			);
		}

		// Cache in RAM
		self.auth_chain_cache
			.lock()
			.expect("locked")
			.insert(key, auth_chain);

		Ok(())
	}
}
