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

pub(crate) use account::*;
pub(crate) use alias::*;
pub(crate) use backup::*;
pub(crate) use capabilities::*;
pub(crate) use config::*;
pub(crate) use context::*;
pub(crate) use device::*;
pub(crate) use directory::*;
pub(crate) use filter::*;
pub(crate) use keys::*;
pub(crate) use media::*;
pub(crate) use membership::*;
pub(crate) use message::*;
pub(crate) use presence::*;
pub(crate) use profile::*;
pub(crate) use push::*;
pub(crate) use read_marker::*;
pub(crate) use redact::*;
pub(crate) use relations::*;
pub(crate) use report::*;
pub(crate) use room::*;
pub(crate) use search::*;
pub(crate) use session::*;
pub(crate) use space::*;
pub(crate) use state::*;
pub(crate) use sync::*;
pub(crate) use tag::*;
pub(crate) use thirdparty::*;
pub(crate) use threads::*;
pub(crate) use to_device::*;
pub(crate) use typing::*;
pub(crate) use unstable::*;
pub(crate) use unversioned::*;
pub(crate) use user_directory::*;
pub(crate) use voip::*;

/// generated device ID length
const DEVICE_ID_LENGTH: usize = 10;

/// generated user access token length
const TOKEN_LENGTH: usize = 32;

/// generated user session ID length
pub(crate) const SESSION_ID_LENGTH: usize = 32;

/// auto-generated password length
pub(crate) const AUTO_GEN_PASSWORD_LENGTH: usize = 25;
