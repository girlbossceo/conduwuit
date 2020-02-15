#![feature(proc_macro_hygiene, decl_macro)]
mod ruma_wrapper;

use {
    rocket::{get, post, routes},
    ruma_client_api::r0::account::register,
    ruma_wrapper::Ruma,
    std::convert::TryInto,
};

#[post("/_matrix/client/r0/register", data = "<body>")]
fn register_route(body: Ruma<register::Request>) -> Ruma<register::Response> {
    Ruma(register::Response {
        access_token: "42".to_owned(),
        home_server: "deprecated".to_owned(),
        user_id: "@yourrequestedid:homeserver.com".try_into().unwrap(),
        device_id: body.device_id.clone().unwrap_or_default(),
    })
}

fn main() {
    pretty_env_logger::init();
    rocket::ignite()
        .mount("/", routes![register_route])
        .launch();
}
