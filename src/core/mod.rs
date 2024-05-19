pub mod alloc;
pub mod config;
pub mod debug;
pub mod error;
pub mod log;
pub mod mods;
pub mod pducount;
pub mod server;
pub mod utils;

pub use config::Config;
pub use error::{Error, Result, RumaResponse};
pub use pducount::PduCount;
pub use server::Server;
pub use utils::conduwuit_version;

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
