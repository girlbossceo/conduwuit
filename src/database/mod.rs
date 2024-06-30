pub mod cork;
mod kvdatabase;
mod kvengine;
mod kvtree;

#[cfg(feature = "rocksdb")]
pub(crate) mod rocksdb;

#[cfg(feature = "rocksdb")]
pub(crate) mod watchers;

extern crate conduit_core as conduit;
pub(crate) use conduit::{Config, Result};
pub use cork::Cork;
pub use kvdatabase::KeyValueDatabase;
pub use kvengine::KeyValueDatabaseEngine;
pub use kvtree::KvTree;

conduit::mod_ctor! {}
conduit::mod_dtor! {}
