pub mod client_server;
pub mod router;
mod ruma_wrapper;
pub mod server_server;

extern crate conduit_core as conduit;
extern crate conduit_service as service;

pub use client_server::membership::{join_room_by_id_helper, leave_all_rooms};
pub(crate) use conduit::{debug_info, debug_warn, utils, Error, Result};
pub(crate) use ruma_wrapper::{Ruma, RumaResponse};
pub(crate) use service::{pdu::PduEvent, services, user_is_local};

conduit::mod_ctor! {}
conduit::mod_dtor! {}
