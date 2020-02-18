#![feature(proc_macro_hygiene, decl_macro)]
mod ruma_wrapper;

use {
    rocket::{get, post, put, routes},
    ruma_client_api::{
        error::{Error, ErrorKind},
        r0::{
            account::register, alias::get_alias, membership::join_room_by_id,
            message::create_message_event,
        },
        unversioned::get_supported_versions,
    },
    ruma_wrapper::{MatrixResult, Ruma},
    std::convert::TryInto,
};

#[get("/_matrix/client/versions")]
fn get_supported_versions_route() -> MatrixResult<get_supported_versions::Response> {
    MatrixResult(Ok(get_supported_versions::Response {
        versions: vec!["r0.6.0".to_owned()],
    }))
}

#[post("/_matrix/client/r0/register", data = "<body>")]
fn register_route(body: Ruma<register::Request>) -> MatrixResult<register::Response> {
    let user_id = match (*format!(
        "@{}:localhost",
        body.username.clone().unwrap_or("randomname".to_owned())
    ))
    .try_into()
    {
        Err(_) => {
            return MatrixResult(Err(Error {
                kind: ErrorKind::InvalidUsername,
                message: "Username was invalid. ".to_owned(),
                status_code: http::StatusCode::BAD_REQUEST,
            }))
        }
        Ok(user_id) => user_id,
    };

    MatrixResult(Ok(register::Response {
        access_token: "randomtoken".to_owned(),
        home_server: "localhost".to_owned(),
        user_id,
        device_id: body.device_id.clone().unwrap_or("randomid".to_owned()),
    }))
}

#[get("/_matrix/client/r0/directory/room/<room_alias>")]
fn get_alias_route(room_alias: String) -> MatrixResult<get_alias::Response> {
    let room_id = match &*room_alias {
        "#room:localhost" => "!xclkjvdlfj:localhost",
        _ => {
            return MatrixResult(Err(Error {
                kind: ErrorKind::NotFound,
                message: "Room not found.".to_owned(),
                status_code: http::StatusCode::NOT_FOUND,
            }))
        }
    }
    .try_into()
    .unwrap();

    MatrixResult(Ok(get_alias::Response {
        room_id,
        servers: vec!["localhost".to_owned()],
    }))
}

#[post("/_matrix/client/r0/rooms/<_room_id>/join", data = "<body>")]
fn join_room_by_id_route(
    _room_id: String,
    body: Ruma<join_room_by_id::Request>,
) -> MatrixResult<join_room_by_id::Response> {
    MatrixResult(Ok(join_room_by_id::Response {
        room_id: body.room_id.clone(),
    }))
}

#[put(
    "/_matrix/client/r0/rooms/<_room_id>/send/<_event_type>/<_txn_id>",
    data = "<body>"
)]
fn create_message_event_route(
    _room_id: String,
    _event_type: String,
    _txn_id: String,
    body: Ruma<create_message_event::IncomingRequest>,
) -> MatrixResult<create_message_event::Response> {
    dbg!(body.0);
    MatrixResult(Ok(create_message_event::Response {
        event_id: "$randomeventid".try_into().unwrap(),
    }))
}

fn main() {
    pretty_env_logger::init();
    rocket::ignite()
        .mount(
            "/",
            routes![
                get_supported_versions_route,
                register_route,
                get_alias_route,
                join_room_by_id_route,
                create_message_event_route,
            ],
        )
        .launch();
}
