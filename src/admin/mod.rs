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
pub(crate) mod user;
pub(crate) mod utils;

extern crate conduit_api as api;
extern crate conduit_core as conduit;
extern crate conduit_service as service;

pub(crate) use conduit::{mod_ctor, mod_dtor, Result};
pub(crate) use service::{services, user_is_local};

pub(crate) use crate::{
	handler::Service,
	utils::{escape_html, get_room_info},
};

mod_ctor! {}
mod_dtor! {}

/// Install the admin command handler
#[allow(clippy::let_underscore_must_use)]
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
#[allow(clippy::let_underscore_must_use)]
pub async fn fini() {
	_ = services().admin.handle.write().await.take();
	_ = services()
		.admin
		.complete
		.write()
		.expect("locked for writing")
		.take();
}

#[cfg(test)]
mod test {
	use clap::Parser;

	use crate::handler::AdminCommand;

	#[test]
	fn get_help_short() { get_help_inner("-h"); }

	#[test]
	fn get_help_long() { get_help_inner("--help"); }

	#[test]
	fn get_help_subcommand() { get_help_inner("help"); }

	fn get_help_inner(input: &str) {
		let error = AdminCommand::try_parse_from(["argv[0] doesn't matter", input])
			.unwrap_err()
			.to_string();

		// Search for a handful of keywords that suggest the help printed properly
		assert!(error.contains("Usage:"));
		assert!(error.contains("Commands:"));
		assert!(error.contains("Options:"));
	}
}
