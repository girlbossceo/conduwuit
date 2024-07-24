#![recursion_limit = "168"]
#![allow(clippy::wildcard_imports)]
#![allow(clippy::enum_glob_use)]

pub(crate) mod admin;
pub(crate) mod handler;
mod tests;
pub(crate) mod utils;

pub(crate) mod appservice;
pub(crate) mod check;
pub(crate) mod debug;
pub(crate) mod federation;
pub(crate) mod media;
pub(crate) mod query;
pub(crate) mod room;
pub(crate) mod server;
pub(crate) mod user;

extern crate conduit_api as api;
extern crate conduit_core as conduit;
extern crate conduit_service as service;

pub(crate) use conduit::Result;
pub(crate) use service::services;

pub(crate) use crate::utils::{escape_html, get_room_info};

pub(crate) const PAGE_SIZE: usize = 100;

conduit::mod_ctor! {}
conduit::mod_dtor! {}
conduit::rustc_flags_capture! {}

/// Install the admin command handler
pub async fn init() {
	_ = services()
		.admin
		.complete
		.write()
		.expect("locked for writing")
		.insert(handler::complete);
	_ = services()
		.admin
		.handle
		.write()
		.await
		.insert(handler::handle);
}

/// Uninstall the admin command handler
pub async fn fini() {
	_ = services().admin.handle.write().await.take();
	_ = services()
		.admin
		.complete
		.write()
		.expect("locked for writing")
		.take();
}
