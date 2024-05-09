pub(crate) mod key_value;
pub mod pdu;
pub mod services;

pub mod account_data;
pub mod admin;
pub mod appservice;
pub mod globals;
pub mod key_backups;
pub mod media;
pub mod presence;
pub mod pusher;
pub mod rooms;
pub mod sending;
pub mod transaction_ids;
pub mod uiaa;
pub mod users;

extern crate conduit_core as conduit;
extern crate conduit_database as database;
use std::sync::RwLock;

pub(crate) use conduit::{config, debug_error, debug_info, debug_warn, utils, Config, Error, PduCount, Result};
pub(crate) use database::KeyValueDatabase;
pub use globals::{server_is_ours, user_is_local};
pub use pdu::PduEvent;
pub use services::Services;

pub(crate) use crate as service;

conduit::mod_ctor! {}
conduit::mod_dtor! {}

pub static SERVICES: RwLock<Option<&'static Services>> = RwLock::new(None);

#[must_use]
pub fn services() -> &'static Services {
	SERVICES
		.read()
		.expect("SERVICES locked for reading")
		.expect("SERVICES initialized with Services instance")
}

#[inline]
pub fn available() -> bool {
	SERVICES
		.read()
		.expect("SERVICES locked for reading")
		.is_some()
}
