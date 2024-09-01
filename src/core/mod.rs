pub mod alloc;
pub mod config;
pub mod debug;
pub mod error;
pub mod info;
pub mod log;
pub mod metrics;
pub mod mods;
pub mod pdu;
pub mod result;
pub mod server;
pub mod utils;

pub use ::toml;
pub use config::Config;
pub use error::Error;
pub use info::{rustc_flags_capture, version, version::version};
pub use pdu::{PduBuilder, PduCount, PduEvent};
pub use result::Result;
pub use server::Server;
pub use utils::{ctor, dtor, implement};

pub use crate as conduit_core;

rustc_flags_capture! {}

#[cfg(not(conduit_mods))]
pub mod mods {
	#[macro_export]
	macro_rules! mod_ctor {
		() => {};
	}
	#[macro_export]
	macro_rules! mod_dtor {
		() => {};
	}
}
