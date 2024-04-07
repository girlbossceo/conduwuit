mod account;
mod alias;
mod backup;
mod capabilities;
mod config;
mod context;
mod device;
mod directory;
mod filter;
mod keys;
mod media;
mod membership;
mod message;
mod presence;
mod profile;
mod push;
mod read_marker;
mod redact;
mod relations;
mod report;
mod room;
mod search;
mod session;
mod space;
mod state;
mod sync;
mod tag;
mod thirdparty;
mod threads;
mod to_device;
mod typing;
mod unstable;
mod unversioned;
mod user_directory;
mod voip;

pub use account::*;
pub use alias::*;
pub use backup::*;
pub use capabilities::*;
pub use config::*;
pub use context::*;
pub use device::*;
pub use directory::*;
pub use filter::*;
pub use keys::*;
pub use media::*;
pub use membership::*;
pub use message::*;
pub use presence::*;
pub use profile::*;
pub use push::*;
pub use read_marker::*;
pub use redact::*;
pub use relations::*;
pub use report::*;
pub use room::*;
pub use search::*;
pub use session::*;
pub use space::*;
pub use state::*;
pub use sync::*;
pub use tag::*;
pub use thirdparty::*;
pub use threads::*;
pub use to_device::*;
pub use typing::*;
pub use unstable::*;
pub use unversioned::*;
pub use user_directory::*;
pub use voip::*;

/// generated device ID length
pub const DEVICE_ID_LENGTH: usize = 10;

/// generated user access token length
pub const TOKEN_LENGTH: usize = 32;

/// generated user session ID length
pub const SESSION_ID_LENGTH: usize = 32;

/// auto-generated password length
pub const AUTO_GEN_PASSWORD_LENGTH: usize = 25;
