pub mod client;
pub mod router;
mod ruma_wrapper;
pub mod server;

extern crate conduit_core as conduit;
extern crate conduit_service as service;

pub(crate) use conduit::{debug_info, debug_warn, utils, Error, Result};
pub(crate) use ruma_wrapper::{Ruma, RumaResponse};
pub(crate) use service::{pdu::PduEvent, services, user_is_local};

conduit::mod_ctor! {}
conduit::mod_dtor! {}
