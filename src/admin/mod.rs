pub(crate) mod appservice;
pub(crate) mod debug;
pub(crate) mod federation;
pub(crate) mod fsck;
pub(crate) mod handler;
pub(crate) mod media;
pub(crate) mod query;
pub(crate) mod room;
pub(crate) mod server;
pub(crate) mod tester;
pub(crate) mod user;
pub(crate) mod utils;

extern crate conduit_api as api;
extern crate conduit_core as conduit;
extern crate conduit_service as service;

pub(crate) use conduit::{mod_ctor, mod_dtor, Result};
pub use handler::handle;
pub(crate) use service::{services, user_is_local};

pub(crate) use crate::{
	handler::Service,
	utils::{escape_html, get_room_info},
};

mod_ctor! {}
mod_dtor! {}

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
