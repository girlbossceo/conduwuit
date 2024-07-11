#![allow(refining_impl_trait)]

mod service;
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

use std::sync::{Arc, RwLock};

pub(crate) use conduit::{config, debug_error, debug_info, debug_warn, utils, Config, Error, Result, Server};
pub use conduit::{pdu, PduBuilder, PduCount, PduEvent};
use database::Database;
pub(crate) use service::{Args, Service};

pub use crate::{
	globals::{server_is_ours, user_is_local},
	services::Services,
};

conduit::mod_ctor! {}
conduit::mod_dtor! {}

static SERVICES: RwLock<Option<&Services>> = RwLock::new(None);

pub async fn init(server: &Arc<Server>) -> Result<()> {
	let d = Arc::new(Database::open(server).await?);
	let s = Box::new(Services::build(server.clone(), d)?);
	_ = SERVICES.write().expect("write locked").insert(Box::leak(s));

	services().start().await
}

pub async fn fini() {
	services().stop().await;

	// Deactivate services(). Any further use will panic the caller.
	let s = SERVICES
		.write()
		.expect("write locked")
		.take()
		.expect("services initialized");

	let s: *mut Services = std::ptr::from_ref(s).cast_mut();
	//SAFETY: Services was instantiated in init() and leaked into the SERVICES
	// global perusing as 'static for the duration of service. Now we reclaim
	// it to drop it before unloading the module. If this is not done there wil
	// be multiple instances after module reload.
	let s = unsafe { Box::from_raw(s) };

	// Drop it so we encounter any trouble before the infolog message
	drop(s);
}

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
