use clap::Subcommand;
#[cfg(feature = "hot_reload")]
#[allow(unused_imports)]
#[allow(clippy::wildcard_imports)]
use hot_lib::*;
use ruma::events::room::message::RoomMessageEventContent;

use crate::{debug_error, Result};

#[cfg(feature = "hot_reload")]
#[hot_lib_reloader::hot_module(dylib = "lib")]
mod hot_lib {
	hot_functions_from_file!("lib/src/lib.rs");
}

#[cfg_attr(test, derive(Debug))]
#[derive(Subcommand)]
pub(crate) enum TestCommands {
	Test1,
}

pub(crate) async fn process(command: TestCommands, _body: Vec<&str>) -> Result<RoomMessageEventContent> {
	Ok(match command {
		TestCommands::Test1 => {
			debug_error!("before calling test_command");
			test_command();
			debug_error!("after calling test_command");
			RoomMessageEventContent::notice_plain(String::from("loaded"))
		},
	})
}
