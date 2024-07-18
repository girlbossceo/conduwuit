#![recursion_limit = "160"]

pub mod client;
pub mod router;
pub mod server;

extern crate conduit_core as conduit;
extern crate conduit_service as service;

pub(crate) use conduit::{debug_info, debug_warn, pdu::PduEvent, utils, Error, Result};
pub(crate) use service::{services, user_is_local};

pub use crate::router::State;
pub(crate) use crate::router::{Ruma, RumaResponse};

conduit::mod_ctor! {}
conduit::mod_dtor! {}
