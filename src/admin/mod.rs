#![recursion_limit = "168"]
#![allow(clippy::wildcard_imports)]

pub(crate) mod appservice;
pub(crate) mod check;
pub(crate) mod debug;
pub(crate) mod federation;
pub(crate) mod handler;
pub(crate) mod media;
pub(crate) mod query;
pub(crate) mod room;
pub(crate) mod server;
mod tests;
pub(crate) mod user;
pub(crate) mod utils;

extern crate conduit_api as api;
extern crate conduit_core as conduit;
extern crate conduit_service as service;

pub(crate) use conduit::{mod_ctor, mod_dtor, Result};
pub(crate) use service::{services, user_is_local};

pub(crate) use crate::utils::{escape_html, get_room_info};

mod_ctor! {}
mod_dtor! {}

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
