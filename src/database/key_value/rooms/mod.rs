mod alias;
mod auth_chain;
mod directory;
mod lazy_load;
mod metadata;
mod outlier;
mod pdu_metadata;
mod read_receipt;
mod search;
mod short;
mod state;
mod state_accessor;
mod state_cache;
mod state_compressor;
mod threads;
mod timeline;
mod user;

use crate::{database::KeyValueDatabase, service};

impl service::rooms::Data for KeyValueDatabase {}
