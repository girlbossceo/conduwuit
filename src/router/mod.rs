#![recursion_limit = "160"]

mod layers;
mod request;
mod router;
mod run;
mod serve;

extern crate conduit_core as conduit;

use std::{future::Future, pin::Pin, sync::Arc};

use conduit::{Result, Server};

conduit::mod_ctor! {}
conduit::mod_dtor! {}

#[no_mangle]
pub extern "Rust" fn start(server: &Arc<Server>) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> {
	Box::pin(run::start(server.clone()))
}

#[no_mangle]
pub extern "Rust" fn stop(server: &Arc<Server>) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> {
	Box::pin(run::stop(server.clone()))
}

#[no_mangle]
pub extern "Rust" fn run(server: &Arc<Server>) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> {
	Box::pin(run::run(server.clone()))
}
