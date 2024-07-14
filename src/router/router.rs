use std::sync::Arc;

use axum::{response::IntoResponse, routing::get, Router};
use conduit::{Error, Server};
use conduit_service as service;
use http::{StatusCode, Uri};
use ruma::api::client::error::ErrorKind;

extern crate conduit_api as api;

pub(crate) fn build(server: &Arc<Server>) -> Router {
	let state = service::services();
	api::router::build(Router::new(), server)
		.route("/", get(it_works))
		.fallback(not_found)
		.with_state(state);

	api::routes::build(router, server)
}

async fn not_found(_uri: Uri) -> impl IntoResponse {
	Error::Request(ErrorKind::Unrecognized, "Not Found".into(), StatusCode::NOT_FOUND)
}

async fn it_works() -> &'static str { "hewwo from conduwuit woof!" }
