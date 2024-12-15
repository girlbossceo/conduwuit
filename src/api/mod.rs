#![allow(clippy::toplevel_ref_arg)]

pub mod client;
pub mod router;
pub mod server;

extern crate conduwuit_core as conduwuit;
extern crate conduwuit_service as service;

pub(crate) use conduwuit::{debug_info, pdu::PduEvent, utils, Error, Result};

pub(crate) use self::router::{Ruma, RumaResponse, State};

conduwuit::mod_ctor! {}
conduwuit::mod_dtor! {}
