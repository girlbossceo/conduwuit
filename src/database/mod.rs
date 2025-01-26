extern crate conduwuit_core as conduwuit;
extern crate rust_rocksdb as rocksdb;

conduwuit::mod_ctor! {}
conduwuit::mod_dtor! {}
conduwuit::rustc_flags_capture! {}

mod cork;
mod de;
mod deserialized;
mod engine;
mod handle;
pub mod keyval;
mod map;
pub mod maps;
mod pool;
mod ser;
mod stream;
#[cfg(test)]
mod tests;
pub(crate) mod util;
mod watchers;

use std::{ops::Index, sync::Arc};

use conduwuit::{err, Result, Server};

pub use self::{
	de::{Ignore, IgnoreAll},
	deserialized::Deserialized,
	handle::Handle,
	keyval::{serialize_key, serialize_val, KeyVal, Slice},
	map::{compact, Get, Map, Qry},
	ser::{serialize, serialize_to, serialize_to_vec, Cbor, Interfix, Json, Separator, SEP},
};
pub(crate) use self::{
	engine::{context::Context, Engine},
	util::or_else,
};
use crate::maps::{Maps, MapsKey, MapsVal};

pub struct Database {
	maps: Maps,
	pub db: Arc<Engine>,
	pub(crate) _ctx: Arc<Context>,
}

impl Database {
	/// Load an existing database or create a new one.
	pub async fn open(server: &Arc<Server>) -> Result<Arc<Self>> {
		let ctx = Context::new(server)?;
		let db = Engine::open(ctx.clone(), maps::MAPS).await?;
		Ok(Arc::new(Self {
			maps: maps::open(&db)?,
			db: db.clone(),
			_ctx: ctx,
		}))
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
