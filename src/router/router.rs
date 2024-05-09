use std::sync::Arc;

use axum::{response::IntoResponse, routing::get, Router};
use conduit::{Error, Server};
use http::Uri;
use ruma::api::client::error::ErrorKind;

extern crate conduit_api as api;

pub(crate) fn build(server: &Arc<Server>) -> Router {
	let router = Router::new().fallback(not_found).route("/", get(it_works));

	api::router::build(router, server)
}

async fn not_found(_uri: Uri) -> impl IntoResponse {
	Error::BadRequest(ErrorKind::Unrecognized, "Unrecognized request")
}

async fn it_works() -> &'static str { "hewwo from conduwuit woof!" }
