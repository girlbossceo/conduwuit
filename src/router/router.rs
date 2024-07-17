use std::sync::Arc;

use axum::{response::IntoResponse, routing::get, Router};
use conduit::{Error, Server};
use http::{StatusCode, Uri};
use ruma::api::client::error::ErrorKind;

extern crate conduit_api as api;
extern crate conduit_service as service;

pub(crate) fn build(server: &Arc<Server>) -> Router {
	let router = Router::<api::State>::new();

	api::router::build(router, server)
		.route("/", get(it_works))
		.fallback(not_found)
		.with_state(service::services())
}

async fn not_found(_uri: Uri) -> impl IntoResponse {
	Error::Request(ErrorKind::Unrecognized, "Not Found".into(), StatusCode::NOT_FOUND)
}

async fn it_works() -> &'static str { "hewwo from conduwuit woof!" }
