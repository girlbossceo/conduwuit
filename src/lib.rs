pub mod client_server;
mod database;
mod error;
mod pdu;
pub mod push_rules;
mod ruma_wrapper;
mod utils;

pub use database::Database;
pub use error::{Error, Result};
pub use pdu::PduEvent;
pub use ruma_wrapper::{ConduitResult, Ruma, RumaResponse};
use std::ops::Deref;

pub struct State<'r, T: Send + Sync + 'static>(&'r T);

impl<'r, T: Send + Sync + 'static> Deref for State<'r, T> {
    type Target = T;

    #[inline(always)]
    fn deref(&self) -> &T {
        self.0
    }
}
