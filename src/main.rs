#![feature(proc_macro_hygiene, decl_macro)]
mod data;
mod database;
mod pdu;
mod ruma_wrapper;
mod utils;

pub use data::Data;
pub use database::Database;
pub use pdu::PduEvent;

use log::debug;
use rocket::{get, options, post, put, routes, State};
use ruma_client_api::{
    error::{Error, ErrorKind},
    r0::{
        account::register,
        alias::get_alias,
        membership::join_room_by_id,
        message::create_message_event,
        room::create_room,
        session::login,
        state::{create_state_event_for_empty_key, create_state_event_for_key},
        sync::sync_events,
    },
    unversioned::get_supported_versions,
};
use ruma_events::EventType;
use ruma_identifiers::{RoomId, UserId};
use ruma_wrapper::{MatrixResult, Ruma};
use serde_json::json;
use std::{collections::HashMap, convert::TryInto, path::PathBuf};

#[get("/_matrix/client/versions")]
fn get_supported_versions_route() -> MatrixResult<get_supported_versions::Response> {
    MatrixResult(Ok(get_supported_versions::Response {
        versions: vec!["r0.6.0".to_owned()],
        unstable_features: HashMap::new(),
    }))
}

#[post("/_matrix/client/r0/register", data = "<body>")]
fn register_route(
    data: State<Data>,
    body: Ruma<register::Request>,
) -> MatrixResult<register::Response> {
    // Validate user id
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

    // Check if username is creative enough
    if data.user_exists(&user_id) {
        debug!("ID already taken");
        return MatrixResult(Err(Error {
            kind: ErrorKind::UserInUse,
            message: "Desired user ID is already taken.".to_owned(),
            status_code: http::StatusCode::BAD_REQUEST,
        }));
    }

    // Create user
    data.user_add(&user_id, body.password.clone());

    // Generate new device id if the user didn't specify one
    let device_id = body
        .device_id
        .clone()
        .unwrap_or_else(|| "TODO:randomdeviceid".to_owned());

    // Add device
    data.device_add(&user_id, &device_id);

    // Generate new token for the device
    let token = "TODO:randomtoken".to_owned();
    data.token_replace(&user_id, &device_id, token.clone());

    MatrixResult(Ok(register::Response {
        access_token: token,
        home_server: data.hostname().to_owned(),
        user_id,
        device_id,
    }))
}

#[post("/_matrix/client/r0/login", data = "<body>")]
fn login_route(data: State<Data>, body: Ruma<login::Request>) -> MatrixResult<login::Response> {
    // Validate login method
    let user_id =
        if let (login::UserInfo::MatrixId(mut username), login::LoginInfo::Password { password }) =
            (body.user.clone(), body.login_info.clone())
        {
            if !username.contains(':') {
                username = format!("@{}:{}", username, data.hostname());
            }
            if let Ok(user_id) = (*username).try_into() {
                if !data.user_exists(&user_id) {}

                // Check password
                if let Some(correct_password) = data.password_get(&user_id) {
                    if password == correct_password {
                        // Success!
                        user_id
                    } else {
                        debug!("Invalid password.");
                        return MatrixResult(Err(Error {
                            kind: ErrorKind::Unknown,
                            message: "".to_owned(),
                            status_code: http::StatusCode::FORBIDDEN,
                        }));
                    }
                } else {
                    debug!("UserId does not exist (has no assigned password). Can't log in.");
                    return MatrixResult(Err(Error {
                        kind: ErrorKind::Forbidden,
                        message: "".to_owned(),
                        status_code: http::StatusCode::FORBIDDEN,
                    }));
                }
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

    // Generate new device id if the user didn't specify one
    let device_id = body
        .device_id
        .clone()
        .unwrap_or("TODO:randomdeviceid".to_owned());

    // Add device
    data.device_add(&user_id, &device_id);

    // Generate a new token for the device
    let token = "TODO:randomtoken".to_owned();
    data.token_replace(&user_id, &device_id, token.clone());

    return MatrixResult(Ok(login::Response {
        user_id,
        access_token: token,
        home_server: Some(data.hostname().to_owned()),
        device_id,
        well_known: None,
    }));
}

#[post("/_matrix/client/r0/createRoom", data = "<body>")]
fn create_room_route(
    data: State<Data>,
    body: Ruma<create_room::Request>,
) -> MatrixResult<create_room::Response> {
    // TODO: check if room is unique
    let room_id = RoomId::new(data.hostname()).expect("host is valid");

    data.room_join(
        &room_id,
        body.user_id.as_ref().expect("user is authenticated"),
    );

    data.pdu_append(
        room_id.clone(),
        body.user_id.clone().expect("user is authenticated"),
        EventType::RoomMessage,
        json!({"msgtype": "m.text", "body": "Hello"}),
        None,
    );

    MatrixResult(Ok(create_room::Response { room_id }))
}

#[get("/_matrix/client/r0/directory/room/<room_alias>")]
fn get_alias_route(room_alias: String) -> MatrixResult<get_alias::Response> {
    // TODO
    let room_id = match &*room_alias {
        "#room:localhost" => "!xclkjvdlfj:localhost",
        _ => {
            debug!("Room not found.");
            return MatrixResult(Err(Error {
                kind: ErrorKind::NotFound,
                message: "Room not found.".to_owned(),
                status_code: http::StatusCode::NOT_FOUND,
            }));
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
    data: State<Data>,
    body: Ruma<join_room_by_id::Request>,
    _room_id: String,
) -> MatrixResult<join_room_by_id::Response> {
    data.room_join(
        &body.room_id,
        body.user_id.as_ref().expect("user is authenticated"),
    );
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
    let event_id = data.pdu_append(
        body.room_id.clone(),
        body.user_id.clone().expect("user is authenticated"),
        body.event_type.clone(),
        body.json_body,
        None,
    );
    MatrixResult(Ok(create_message_event::Response { event_id }))
}

#[put(
    "/_matrix/client/r0/rooms/<_room_id>/state/<_event_type>/<_state_key>",
    data = "<body>"
)]
fn create_state_event_for_key_route(
    data: State<Data>,
    _room_id: String,
    _event_type: String,
    _state_key: String,
    body: Ruma<create_state_event_for_key::Request>,
) -> MatrixResult<create_state_event_for_key::Response> {
    // Reponse of with/without key is the same
    let event_id = data.pdu_append(
        body.room_id.clone(),
        body.user_id.clone().expect("user is authenticated"),
        body.event_type.clone(),
        body.json_body.clone(),
        Some(body.state_key.clone()),
    );
    MatrixResult(Ok(create_state_event_for_key::Response { event_id }))
}

#[put(
    "/_matrix/client/r0/rooms/<_room_id>/state/<_event_type>",
    data = "<body>"
)]
fn create_state_event_for_empty_key_route(
    data: State<Data>,
    _room_id: String,
    _event_type: String,
    body: Ruma<create_state_event_for_empty_key::Request>,
) -> MatrixResult<create_state_event_for_empty_key::Response> {
    // Reponse of with/without key is the same
    let event_id = data.pdu_append(
        body.room_id.clone(),
        body.user_id.clone().expect("user is authenticated"),
        body.event_type.clone(),
        body.json_body,
        Some("".to_owned()),
    );
    MatrixResult(Ok(create_state_event_for_empty_key::Response { event_id }))
}

#[get("/_matrix/client/r0/sync", data = "<body>")]
fn sync_route(
    data: State<Data>,
    body: Ruma<sync_events::Request>,
) -> MatrixResult<sync_events::Response> {
    let mut joined_rooms = HashMap::new();
    let joined_roomids = data.rooms_joined(body.user_id.as_ref().expect("user is authenticated"));
    for room_id in joined_roomids {
        let room_events = data
            .pdus_all(&room_id)
            .into_iter()
            .map(|pdu| pdu.to_room_event())
            .collect();

        joined_rooms.insert(
            "!roomid:localhost".try_into().unwrap(),
            sync_events::JoinedRoom {
                account_data: sync_events::AccountData { events: Vec::new() },
                summary: sync_events::RoomSummary {
                    heroes: Vec::new(),
                    joined_member_count: None,
                    invited_member_count: None,
                },
                unread_notifications: sync_events::UnreadNotificationsCount {
                    highlight_count: None,
                    notification_count: None,
                },
                timeline: sync_events::Timeline {
                    limited: Some(false),
                    prev_batch: Some("".to_owned()),
                    events: room_events,
                },
                state: sync_events::State { events: Vec::new() },
                ephemeral: sync_events::Ephemeral { events: Vec::new() },
            },
        );
    }

    MatrixResult(Ok(sync_events::Response {
        next_batch: String::new(),
        rooms: sync_events::Rooms {
            leave: Default::default(),
            join: joined_rooms,
            invite: Default::default(),
        },
        presence: sync_events::Presence { events: Vec::new() },
        device_lists: Default::default(),
        device_one_time_keys_count: Default::default(),
        to_device: sync_events::ToDevice { events: Vec::new() },
    }))
}

#[options("/<_segments..>")]
fn options_route(_segments: PathBuf) -> MatrixResult<create_message_event::Response> {
    MatrixResult(Err(Error {
        kind: ErrorKind::NotFound,
        message: "This is the options route.".to_owned(),
        status_code: http::StatusCode::NOT_FOUND,
    }))
}

fn main() {
    // Log info by default
    if let Err(_) = std::env::var("RUST_LOG") {
        std::env::set_var("RUST_LOG", "matrixserver=debug,info");
    }
    pretty_env_logger::init();

    let data = Data::load_or_create("localhost");
    data.debug();

    rocket::ignite()
        .mount(
            "/",
            routes![
                get_supported_versions_route,
                register_route,
                login_route,
                create_room_route,
                get_alias_route,
                join_room_by_id_route,
                create_message_event_route,
                create_state_event_for_key_route,
                create_state_event_for_empty_key_route,
                sync_route,
                options_route,
            ],
        )
        .manage(data)
        .launch()
        .unwrap();
}
