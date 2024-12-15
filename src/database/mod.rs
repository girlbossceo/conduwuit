mod cork;
mod database;
mod de;
mod deserialized;
mod engine;
mod handle;
pub mod keyval;
mod map;
pub mod maps;
mod opts;
mod pool;
mod ser;
mod stream;
mod tests;
pub(crate) mod util;
mod watchers;

pub(crate) use self::{
	engine::Engine,
	util::{or_else, result},
};

extern crate conduwuit_core as conduwuit;
extern crate rust_rocksdb as rocksdb;

pub use self::{
	database::Database,
	de::{Ignore, IgnoreAll},
	deserialized::Deserialized,
	handle::Handle,
	keyval::{serialize_key, serialize_val, KeyVal, Slice},
	map::Map,
	ser::{serialize, serialize_to, serialize_to_vec, Interfix, Json, Separator, SEP},
};

conduwuit::mod_ctor! {}
conduwuit::mod_dtor! {}
conduwuit::rustc_flags_capture! {}
