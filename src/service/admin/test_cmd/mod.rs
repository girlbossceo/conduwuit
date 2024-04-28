//! test commands generally used for hot lib reloadable functions.
//! see https://github.com/rksm/hot-lib-reloader-rs?tab=readme-ov-file#usage for more details if you are a dev

//#[cfg(not(feature = "hot_reload"))]
//#[allow(unused_imports)]
//#[allow(clippy::wildcard_imports)]
// non hot reloadable functions (?)
//use hot_lib::*;
#[cfg(feature = "hot_reload")]
#[allow(unused_imports)]
#[allow(clippy::wildcard_imports)]
use hot_lib_funcs::*;
use ruma::events::room::message::RoomMessageEventContent;

use crate::{debug_error, Result};

#[cfg(feature = "hot_reload")]
#[hot_lib_reloader::hot_module(dylib = "lib")]
mod hot_lib_funcs {
	// these will be functions from lib.rs, so `use hot_lib_funcs::test_command;`
	hot_functions_from_file!("hot_lib/src/lib.rs");
}

#[cfg_attr(test, derive(Debug))]
#[derive(clap::Subcommand)]
pub(crate) enum TestCommands {
	// !admin test test1
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
