#![warn(
    rust_2018_idioms,
    unused_qualifications,
    clippy::cloned_instead_of_copied,
    clippy::str_to_string
)]
#![allow(clippy::suspicious_else_formatting)]
#![deny(clippy::dbg_macro)]

pub mod appservice_server;
pub mod client_server;
mod database;
mod error;
mod pdu;
mod ruma_wrapper;
pub mod server_server;
mod utils;

pub use database::{Config, Database};
pub use error::{Error, Result};
pub use pdu::PduEvent;
pub use rocket::Config as RocketConfig;
pub use ruma_wrapper::{ConduitResult, Ruma, RumaResponse};
use std::ops::Deref;

pub struct State<'r, T: Send + Sync + 'static>(pub &'r T);

impl<'r, T: Send + Sync + 'static> Deref for State<'r, T> {
    type Target = T;

    #[inline(always)]
    fn deref(&self) -> &T {
        self.0
    }
}
