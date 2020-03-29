#![feature(proc_macro_hygiene, decl_macro)]
mod data;
mod ruma_wrapper;
mod utils;

pub use data::Data;
use log::debug;
use rocket::{get, post, put, routes, State};
use ruma_client_api::{
    error::{Error, ErrorKind},
    r0::{
        account::register, alias::get_alias, membership::join_room_by_id,
        message::create_message_event, session::login,
    },
    unversioned::get_supported_versions,
};
use ruma_events::room::message::MessageEvent;
use ruma_identifiers::{EventId, UserId};
use ruma_wrapper::{MatrixResult, Ruma};
use std::convert::TryFrom;
use std::{collections::HashMap, convert::TryInto};

#[get("/_matrix/client/versions")]
fn get_supported_versions_route() -> MatrixResult<get_supported_versions::Response> {
    MatrixResult(Ok(get_supported_versions::Response {
        versions: vec![
            "r0.0.1".to_owned(),
            "r0.1.0".to_owned(),
            "r0.2.0".to_owned(),
            "r0.3.0".to_owned(),
            "r0.4.0".to_owned(),
            "r0.5.0".to_owned(),
            "r0.6.0".to_owned(),
        ],
        unstable_features: HashMap::new(),
    }))
}

#[post("/_matrix/client/r0/register", data = "<body>")]
fn register_route(
    data: State<Data>,
    body: Ruma<register::Request>,
) -> MatrixResult<register::Response> {
    let user_id: UserId = match (*format!(
        "@{}:{}",
        body.username.clone().unwrap_or("randomname".to_owned()),
        data.hostname()
    ))
    .try_into()
    {
        Err(_) => {
            debug!("Username invalid");
            return MatrixResult(Err(Error {
                kind: ErrorKind::InvalidUsername,
                message: "Username was invalid.".to_owned(),
                status_code: http::StatusCode::BAD_REQUEST,
            }));
        }
        Ok(user_id) => user_id,
    };

    if data.user_exists(&user_id) {
        debug!("ID already taken");
        return MatrixResult(Err(Error {
            kind: ErrorKind::UserInUse,
            message: "Desired user ID is already taken.".to_owned(),
            status_code: http::StatusCode::BAD_REQUEST,
        }));
    }

    data.user_add(user_id.clone(), body.password.clone());

    MatrixResult(Ok(register::Response {
        access_token: "randomtoken".to_owned(),
        home_server: data.hostname(),
        user_id,
        device_id: body.device_id.clone().unwrap_or("randomid".to_owned()),
    }))
}

#[post("/_matrix/client/r0/login", data = "<body>")]
fn login_route(data: State<Data>, body: Ruma<login::Request>) -> MatrixResult<login::Response> {
    let username = if let login::UserInfo::MatrixId(mut username) = body.user.clone() {
        if !username.contains(':') {
            username = format!("@{}:{}", username, data.hostname());
        }
        if let Ok(user_id) = (*username).try_into() {
            if !data.user_exists(&user_id) {
                debug!("Userid does not exist. Can't log in.");
                return MatrixResult(Err(Error {
                    kind: ErrorKind::Forbidden,
                    message: "UserId not found.".to_owned(),
                    status_code: http::StatusCode::BAD_REQUEST,
                }));
            }
            user_id
        } else {
            debug!("Invalid UserId.");
            return MatrixResult(Err(Error {
                kind: ErrorKind::Unknown,
                message: "Bad login type.".to_owned(),
                status_code: http::StatusCode::BAD_REQUEST,
            }));
        }
    } else {
        debug!("Bad login type");
        return MatrixResult(Err(Error {
            kind: ErrorKind::Unknown,
            message: "Bad login type.".to_owned(),
            status_code: http::StatusCode::BAD_REQUEST,
        }));
    };

    return MatrixResult(Ok(login::Response {
        user_id: username.try_into().unwrap(), // Unwrap is okay because the user is already registered
        access_token: "randomtoken".to_owned(),
        home_server: Some("localhost".to_owned()),
        device_id: body.device_id.clone().unwrap_or("randomid".to_owned()),
        well_known: None,
    }));
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
    data: State<Data>,
    _room_id: String,
    _event_type: String,
    _txn_id: String,
    body: Ruma<create_message_event::Request>,
) -> MatrixResult<create_message_event::Response> {
    dbg!(&body);
    if let Ok(content) = body.data.clone().into_result() {
        data.room_event_add(
            &MessageEvent {
                content,
                event_id: EventId::try_from("$randomeventid:localhost").unwrap(),
                origin_server_ts: utils::millis_since_unix_epoch(),
                room_id: Some(body.room_id.clone()),
                sender: UserId::try_from("@TODO:localhost").unwrap(),
                unsigned: None,
            }
            .into(),
        );
    }
    MatrixResult(Ok(create_message_event::Response {
        event_id: "$randomeventid:localhost".try_into().unwrap(),
    }))
}

fn main() {
    // Log info by default
    if let Err(_) = std::env::var("RUST_LOG") {
        std::env::set_var("RUST_LOG", "info");
    }
    pretty_env_logger::init();

    let data = Data::load_or_create();
    data.set_hostname("localhost");

    rocket::ignite()
        .mount(
            "/",
            routes![
                get_supported_versions_route,
                register_route,
                login_route,
                get_alias_route,
                join_room_by_id_route,
                create_message_event_route,
            ],
        )
        .manage(data)
        .launch();
}
