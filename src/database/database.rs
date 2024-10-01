use std::{ops::Index, sync::Arc};

use conduit::{Result, Server};

use crate::{
	cork::Cork,
	maps,
	maps::{Maps, MapsKey, MapsVal},
	Engine, Map,
};

pub struct Database {
	pub db: Arc<Engine>,
	map: Maps,
}

impl Database {
	/// Load an existing database or create a new one.
	pub async fn open(server: &Arc<Server>) -> Result<Arc<Self>> {
		let db = Engine::open(server)?;
		Ok(Arc::new(Self {
			db: db.clone(),
			map: maps::open(&db)?,
		}))
	}

	#[inline]
	#[must_use]
	pub fn cork(&self) -> Cork { Cork::new(&self.db, false, false) }

	#[inline]
	#[must_use]
	pub fn cork_and_flush(&self) -> Cork { Cork::new(&self.db, true, false) }

	#[inline]
	#[must_use]
	pub fn cork_and_sync(&self) -> Cork { Cork::new(&self.db, true, true) }

	#[inline]
	pub fn iter_maps(&self) -> impl Iterator<Item = (&MapsKey, &MapsVal)> + Send + '_ { self.map.iter() }

	#[inline]
	#[must_use]
	pub fn is_read_only(&self) -> bool { self.db.secondary || self.db.read_only }

	#[inline]
	#[must_use]
	pub fn is_secondary(&self) -> bool { self.db.secondary }
}

impl Index<&str> for Database {
	type Output = Arc<Map>;

	fn index(&self, name: &str) -> &Self::Output {
		self.map
			.get(name)
			.expect("column in database does not exist")
	}
}
