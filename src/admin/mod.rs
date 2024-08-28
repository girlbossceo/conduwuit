#![recursion_limit = "192"]
#![allow(clippy::wildcard_imports)]
#![allow(clippy::enum_glob_use)]

pub(crate) mod admin;
pub(crate) mod command;
pub(crate) mod processor;
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
pub(crate) use conduit_macros::{admin_command, admin_command_dispatch};

pub(crate) use crate::{
	command::Command,
	utils::{escape_html, get_room_info},
};

pub(crate) const PAGE_SIZE: usize = 100;

conduit::mod_ctor! {}
conduit::mod_dtor! {}
conduit::rustc_flags_capture! {}

/// Install the admin command processor
pub async fn init(admin_service: &service::admin::Service) {
	_ = admin_service
		.complete
		.write()
		.expect("locked for writing")
		.insert(processor::complete);
	_ = admin_service
		.handle
		.write()
		.await
		.insert(processor::dispatch);
}

/// Uninstall the admin command handler
pub async fn fini(admin_service: &service::admin::Service) {
	_ = admin_service.handle.write().await.take();
	_ = admin_service
		.complete
		.write()
		.expect("locked for writing")
		.take();
}
