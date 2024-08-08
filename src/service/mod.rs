#![recursion_limit = "192"]
#![allow(refining_impl_trait)]

mod manager;
mod service;
pub mod services;

pub mod account_data;
pub mod admin;
pub mod appservice;
pub mod client;
pub mod emergency;
pub mod globals;
pub mod key_backups;
pub mod media;
pub mod presence;
pub mod pusher;
pub mod resolver;
pub mod rooms;
pub mod sending;
pub mod server_keys;
pub mod sync;
pub mod transaction_ids;
pub mod uiaa;
pub mod updates;
pub mod users;

extern crate conduit_core as conduit;
extern crate conduit_database as database;

pub use conduit::{pdu, PduBuilder, PduCount, PduEvent};
pub(crate) use service::{Args, Dep, Service};

pub use crate::services::Services;

conduit::mod_ctor! {}
conduit::mod_dtor! {}
conduit::rustc_flags_capture! {}
