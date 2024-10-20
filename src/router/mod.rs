mod layers;
mod request;
mod router;
mod run;
mod serve;

extern crate conduit_core as conduit;

use std::{future::Future, pin::Pin, sync::Arc};

use conduit::{Result, Server};
use conduit_service::Services;

conduit::mod_ctor! {}
conduit::mod_dtor! {}
conduit::rustc_flags_capture! {}

#[no_mangle]
pub extern "Rust" fn start(server: &Arc<Server>) -> Pin<Box<dyn Future<Output = Result<Arc<Services>>> + Send>> {
	Box::pin(run::start(server.clone()))
}

#[no_mangle]
pub extern "Rust" fn stop(services: Arc<Services>) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> {
	Box::pin(run::stop(services))
}

#[no_mangle]
pub extern "Rust" fn run(services: &Arc<Services>) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> {
	Box::pin(run::run(services.clone()))
}
