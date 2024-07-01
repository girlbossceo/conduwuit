use std::{
	collections::{BTreeMap, HashMap},
	ops::Index,
	sync::{Arc, Mutex, RwLock},
};

use conduit::{PduCount, Result, Server};
use lru_cache::LruCache;
use ruma::{CanonicalJsonValue, OwnedDeviceId, OwnedRoomId, OwnedUserId};

use crate::{cork::Cork, maps, maps::Maps, Engine, Map};

pub struct Database {
	pub db: Arc<Engine>,
	pub map: Maps,

	//TODO: not a database
	pub userdevicesessionid_uiaarequest: RwLock<BTreeMap<(OwnedUserId, OwnedDeviceId, String), CanonicalJsonValue>>,
	pub auth_chain_cache: Mutex<LruCache<Vec<u64>, Arc<[u64]>>>,
	pub appservice_in_room_cache: RwLock<HashMap<OwnedRoomId, HashMap<String, bool>>>,
	pub lasttimelinecount_cache: Mutex<HashMap<OwnedRoomId, PduCount>>,
}

impl Database {
	/// Load an existing database or create a new one.
	pub async fn open(server: &Arc<Server>) -> Result<Self> {
		let config = &server.config;
		let db = Engine::open(server)?;
		Ok(Self {
			db: db.clone(),
			map: maps::open(&db)?,

			userdevicesessionid_uiaarequest: RwLock::new(BTreeMap::new()),
			appservice_in_room_cache: RwLock::new(HashMap::new()),
			lasttimelinecount_cache: Mutex::new(HashMap::new()),
			auth_chain_cache: Mutex::new(LruCache::new(
				(f64::from(config.auth_chain_cache_capacity) * config.conduit_cache_capacity_modifier) as usize,
			)),
		})
	}

	#[must_use]
	pub fn cork(&self) -> Cork { Cork::new(&self.db, false, false) }

	#[must_use]
	pub fn cork_and_flush(&self) -> Cork { Cork::new(&self.db, true, false) }

	#[must_use]
	pub fn cork_and_sync(&self) -> Cork { Cork::new(&self.db, true, true) }
}

impl Index<&str> for Database {
	type Output = Arc<Map>;

	fn index(&self, name: &str) -> &Self::Output {
		self.map
			.get(name)
			.expect("column in database does not exist")
	}
}
