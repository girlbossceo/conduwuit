use std::{ops::Index, sync::Arc};

use conduwuit::{err, Result, Server};

use crate::{
	maps,
	maps::{Maps, MapsKey, MapsVal},
	Engine, Map,
};

pub struct Database {
	pub db: Arc<Engine>,
	maps: Maps,
}

impl Database {
	/// Load an existing database or create a new one.
	pub async fn open(server: &Arc<Server>) -> Result<Arc<Self>> {
		let db = Engine::open(server).await?;
		Ok(Arc::new(Self { db: db.clone(), maps: maps::open(&db)? }))
	}

	#[inline]
	pub fn get(&self, name: &str) -> Result<&Arc<Map>> {
		self.maps
			.get(name)
			.ok_or_else(|| err!(Request(NotFound("column not found"))))
	}

	#[inline]
	pub fn iter(&self) -> impl Iterator<Item = (&MapsKey, &MapsVal)> + Send + '_ {
		self.maps.iter()
	}

	#[inline]
	pub fn keys(&self) -> impl Iterator<Item = &MapsKey> + Send + '_ { self.maps.keys() }

	#[inline]
	#[must_use]
	pub fn is_read_only(&self) -> bool { self.db.is_read_only() }

	#[inline]
	#[must_use]
	pub fn is_secondary(&self) -> bool { self.db.is_secondary() }
}

impl Index<&str> for Database {
	type Output = Arc<Map>;

	fn index(&self, name: &str) -> &Self::Output {
		self.maps
			.get(name)
			.expect("column in database does not exist")
	}
}
