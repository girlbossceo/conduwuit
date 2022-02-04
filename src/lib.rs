#![warn(
    rust_2018_idioms,
    unused_qualifications,
    clippy::cloned_instead_of_copied,
    clippy::str_to_string
)]
#![allow(clippy::suspicious_else_formatting)]
#![deny(clippy::dbg_macro)]

use std::ops::Deref;

mod config;
mod database;
mod error;
mod pdu;
mod ruma_wrapper;
mod utils;

pub mod appservice_server;
pub mod client_server;
pub mod server_server;

pub use config::Config;
pub use database::Database;
pub use error::{Error, Result};
pub use pdu::PduEvent;
pub use rocket::Config as RocketConfig;
pub use ruma_wrapper::{ConduitResult, Ruma, RumaResponse};

pub struct State<'r, T: Send + Sync + 'static>(pub &'r T);

impl<'r, T: Send + Sync + 'static> Deref for State<'r, T> {
    type Target = T;

    #[inline(always)]
    fn deref(&self) -> &T {
        self.0
    }
}
