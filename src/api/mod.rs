pub mod client;
mod router;
pub mod routes;
pub mod server;

extern crate conduit_core as conduit;
extern crate conduit_service as service;

pub(crate) use conduit::{debug_info, debug_warn, utils, Error, Result};
pub(crate) use service::{pdu::PduEvent, services, user_is_local};

pub(crate) use crate::router::{Ruma, RumaResponse};

conduit::mod_ctor! {}
conduit::mod_dtor! {}
