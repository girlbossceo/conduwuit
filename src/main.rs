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
        account::{
            register, AuthenticationFlow, UserInteractiveAuthenticationInfo,
            UserInteractiveAuthenticationResponse,
        },
        alias::get_alias,
        config::{get_global_account_data, set_global_account_data},
        directory::{self, get_public_rooms_filtered},
        filter::{self, create_filter, get_filter},
        keys::{get_keys, upload_keys},
        membership::{join_room_by_id, join_room_by_id_or_alias},
        message::create_message_event,
        presence::set_presence,
        profile::{
            get_avatar_url, get_display_name, get_profile, set_avatar_url, set_display_name,
        },
        push::get_pushrules_all,
        room::create_room,
        session::{get_login_types, login},
        state::{create_state_event_for_empty_key, create_state_event_for_key},
        sync::sync_events,
        thirdparty::get_protocols,
    },
    unversioned::get_supported_versions,
};
use ruma_events::{collections::only::Event, EventType};
use ruma_identifiers::{RoomId, RoomIdOrAliasId, UserId};
use ruma_wrapper::{MatrixResult, Ruma};
use serde_json::json;
use std::{
    collections::HashMap,
    convert::{TryFrom, TryInto},
    path::PathBuf,
    time::Duration,
};

const GUEST_NAME_LENGTH: usize = 10;
const DEVICE_ID_LENGTH: usize = 10;
const SESSION_ID_LENGTH: usize = 256;
const TOKEN_LENGTH: usize = 256;

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
) -> MatrixResult<register::Response, UserInteractiveAuthenticationResponse> {
    if body.auth.is_none() {
        return MatrixResult(Err(UserInteractiveAuthenticationResponse::AuthResponse(
            UserInteractiveAuthenticationInfo {
                flows: vec![AuthenticationFlow {
                    stages: vec!["m.login.dummy".to_owned()],
                }],
                completed: vec![],
                params: json!({}),
                session: Some(utils::random_string(SESSION_ID_LENGTH)),
            },
        )));
    }

    // Validate user id
    let user_id: UserId = match (*format!(
        "@{}:{}",
        body.username
            .clone()
            .unwrap_or_else(|| utils::random_string(GUEST_NAME_LENGTH)),
        data.hostname()
    ))
    .try_into()
    {
        Err(_) => {
            debug!("Username invalid");
            return MatrixResult(Err(UserInteractiveAuthenticationResponse::MatrixError(
                Error {
                    kind: ErrorKind::InvalidUsername,
                    message: "Username was invalid.".to_owned(),
                    status_code: http::StatusCode::BAD_REQUEST,
                },
            )));
        }
        Ok(user_id) => user_id,
    };

    // Check if username is creative enough
    if data.user_exists(&user_id) {
        debug!("ID already taken");
        return MatrixResult(Err(UserInteractiveAuthenticationResponse::MatrixError(
            Error {
                kind: ErrorKind::UserInUse,
                message: "Desired user ID is already taken.".to_owned(),
                status_code: http::StatusCode::BAD_REQUEST,
            },
        )));
    }

    // Create user
    data.user_add(&user_id, body.password.clone());

    // Generate new device id if the user didn't specify one
    let device_id = body
        .device_id
        .clone()
        .unwrap_or_else(|| utils::random_string(DEVICE_ID_LENGTH));

    // Add device
    data.device_add(&user_id, &device_id);

    // Generate new token for the device
    let token = utils::random_string(TOKEN_LENGTH);
    data.token_replace(&user_id, &device_id, token.clone());

    MatrixResult(Ok(register::Response {
        access_token: Some(token),
        user_id,
        device_id: Some(device_id),
    }))
}

#[get("/_matrix/client/r0/login", data = "<_body>")]
fn get_login_route(
    _body: Ruma<get_login_types::Request>,
) -> MatrixResult<get_login_types::Response> {
    MatrixResult(Ok(get_login_types::Response {
        flows: vec![get_login_types::LoginType::Password],
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
                            kind: ErrorKind::Forbidden,
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
                    kind: ErrorKind::InvalidUsername,
                    message: "Bad user id.".to_owned(),
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
        .unwrap_or_else(|| utils::random_string(DEVICE_ID_LENGTH));

    // Add device
    data.device_add(&user_id, &device_id);

    // Generate a new token for the device
    let token = utils::random_string(TOKEN_LENGTH);
    data.token_replace(&user_id, &device_id, token.clone());

    MatrixResult(Ok(login::Response {
        user_id,
        access_token: token,
        home_server: Some(data.hostname().to_owned()),
        device_id,
        well_known: None,
    }))
}

#[get("/_matrix/client/r0/pushrules")]
fn get_pushrules_all_route() -> MatrixResult<get_pushrules_all::Response> {
    // TODO
    MatrixResult(Ok(get_pushrules_all::Response {
        global: HashMap::new(),
    }))
}

#[get(
    "/_matrix/client/r0/user/<_user_id>/filter/<_filter_id>",
    data = "<body>"
)]
fn get_filter_route(
    body: Ruma<get_filter::Request>,
    _user_id: String,
    _filter_id: String,
) -> MatrixResult<get_filter::Response> {
    // TODO
    MatrixResult(Ok(get_filter::Response {
        filter: filter::FilterDefinition {
            event_fields: None,
            event_format: None,
            account_data: None,
            room: None,
            presence: None,
        },
    }))
}

#[post("/_matrix/client/r0/user/<_user_id>/filter", data = "<body>")]
fn create_filter_route(
    body: Ruma<create_filter::Request>,
    _user_id: String,
) -> MatrixResult<create_filter::Response> {
    // TODO
    MatrixResult(Ok(create_filter::Response {
        filter_id: utils::random_string(10),
    }))
}

#[put(
    "/_matrix/client/r0/user/<_user_id>/account_data/<_type>",
    data = "<body>"
)]
fn set_global_account_data_route(
    body: Ruma<set_global_account_data::Request>,
    _user_id: String,
    _type: String,
) -> MatrixResult<set_global_account_data::Response> {
    // TODO
    MatrixResult(Ok(set_global_account_data::Response))
}

#[get(
    "/_matrix/client/r0/user/<_user_id>/account_data/<_type>",
    data = "<body>"
)]
fn get_global_account_data_route(
    body: Ruma<get_global_account_data::Request>,
    _user_id: String,
    _type: String,
) -> MatrixResult<get_global_account_data::Response> {
    // TODO
    MatrixResult(Err(Error {
        kind: ErrorKind::NotFound,
        message: "Data not found.".to_owned(),
        status_code: http::StatusCode::NOT_FOUND,
    }))
}

#[put("/_matrix/client/r0/profile/<_user_id>/displayname", data = "<body>")]
fn set_displayname_route(
    data: State<Data>,
    body: Ruma<set_display_name::Request>,
    _user_id: String,
) -> MatrixResult<set_display_name::Response> {
    let user_id = body.user_id.clone().expect("user is authenticated");

    // Send error on None
    // Synapse returns a parsing error but the spec doesn't require this
    if body.displayname.is_none() {
        debug!("Request was missing the displayname payload.");
        return MatrixResult(Err(Error {
            kind: ErrorKind::MissingParam,
            message: "Missing displayname.".to_owned(),
            status_code: http::StatusCode::BAD_REQUEST,
        }));
    }

    if let Some(displayname) = body.displayname {
        // Some("") will clear the displayname
        if displayname == "" {
            data.displayname_remove(&user_id);
        } else {
            data.displayname_set(&user_id, body.displayname.clone());
            // TODO send a new m.room.member join event with the updated displayname
            // TODO send a new m.presence event with the updated displayname
        }
    }

    MatrixResult(Ok(set_display_name::Response))
}

#[get(
    "/_matrix/client/r0/profile/<user_id_raw>/displayname",
    data = "<body>"
)]
fn get_displayname_route(
    data: State<Data>,
    body: Ruma<get_display_name::Request>,
    user_id_raw: String,
) -> MatrixResult<get_display_name::Response> {
    let user_id = (*body).user_id.clone();
    if !data.user_exists(&user_id) {
        // Return 404 if we don't have a profile for this id
        debug!("Profile was not found.");
        MatrixResult(Err(Error {
            kind: ErrorKind::NotFound,
            message: "Profile was not found.".to_owned(),
            status_code: http::StatusCode::NOT_FOUND,
        }))
    }
    if let Some(displayname) = data.displayname_get(&user_id) {
        return MatrixResult(Ok(get_display_name::Response {
            displayname: Some(displayname),
        }));
    }

    // The user has no displayname
    MatrixResult(Ok(get_display_name::Response { displayname: None }))
}

#[put("/_matrix/client/r0/profile/<_user_id>/avatar_url", data = "<body>")]
fn set_avatar_url_route(
    data: State<Data>,
    body: Ruma<set_avatar_url::Request>,
    _user_id: String,
) -> MatrixResult<set_avatar_url::Response> {
    let user_id = body.user_id.clone().expect("user is authenticated");

    if !body.avatar_url.starts_with("mxc://") {
        debug!("Request contains an invalid avatar_url.");
        return MatrixResult(Err(Error {
            kind: ErrorKind::InvalidParam,
            message: "avatar_url has to start with mxc://.".to_owned(),
            status_code: http::StatusCode::BAD_REQUEST,
        }));
    }

    // TODO in the future when we can handle media uploads make sure that this url is our own server
    // TODO also make sure this is valid mxc:// format (not only starting with it)

    if body.avatar_url == "" {
        data.avatar_url_remove(&user_id);
    } else {
        data.avatar_url_set(&user_id, body.displayname.clone());
        // TODO send a new m.room.member join event with the updated avatar_url
        // TODO send a new m.presence event with the updated avatar_url
    }

    MatrixResult(Ok(set_avatar_url::Response))
}

#[get("/_matrix/client/r0/profile/<user_id_raw>/avatar_url", data = "<body>")]
fn get_avatar_url_route(
    data: State<Data>,
    body: Ruma<get_avatar_url::Request>,
    user_id_raw: String,
) -> MatrixResult<get_avatar_url::Response> {
    let user_id = (*body).user_id.clone();
    if !data.user_exists(&user_id) {
        // Return 404 if we don't have a profile for this id
        debug!("Profile was not found.");
        MatrixResult(Err(Error {
            kind: ErrorKind::NotFound,
            message: "Profile was not found.".to_owned(),
            status_code: http::StatusCode::NOT_FOUND,
        }))
    }
    if let Some(avatar_url) = data.avatar_url_get(&user_id) {
        return MatrixResult(Ok(get_avatar_url::Response {
            avatar_url: Some(avatar_url),
        }));
    }

    // The user has no avatar
    MatrixResult(Ok(get_avatar_url::Response { avatar_url: None }))
}

#[get("/_matrix/client/r0/profile/<user_id_raw>", data = "<body>")]
fn get_profile_route(
    data: State<Data>,
    body: Ruma<get_profile::Request>,
    user_id_raw: String,
) -> MatrixResult<get_profile::Response> {
    let user_id = (*body).user_id.clone();
    let avatar_url = data.avatar_url_get(&user_id);
    let displayname = data.displayname_get(&user_id);

    if avatar_url.is_some() || displayname.is_some() {
        return MatrixResult(Ok(get_profile::Response {
            avatar_url,
            displayname,
        }));
    }

    // Return 404 if we don't have a profile for this id
    debug!("Profile was not found.");
    MatrixResult(Err(Error {
        kind: ErrorKind::NotFound,
        message: "Profile was not found.".to_owned(),
        status_code: http::StatusCode::NOT_FOUND,
    }))
}

#[put("/_matrix/client/r0/presence/<_user_id>/status", data = "<body>")]
fn set_presence_route(
    body: Ruma<set_presence::Request>,
    _user_id: String,
) -> MatrixResult<set_presence::Response> {
    // TODO
    MatrixResult(Ok(set_presence::Response))
}

#[post("/_matrix/client/r0/keys/query", data = "<body>")]
fn get_keys_route(body: Ruma<get_keys::Request>) -> MatrixResult<get_keys::Response> {
    // TODO
    MatrixResult(Ok(get_keys::Response {
        failures: HashMap::new(),
        device_keys: HashMap::new(),
    }))
}

#[post("/_matrix/client/r0/keys/upload", data = "<body>")]
fn upload_keys_route(
    data: State<Data>,
    body: Ruma<upload_keys::Request>,
) -> MatrixResult<upload_keys::Response> {
    MatrixResult(Ok(upload_keys::Response {
        one_time_key_counts: HashMap::new(),
    }))
}

#[post("/_matrix/client/r0/createRoom", data = "<body>")]
fn create_room_route(
    data: State<Data>,
    body: Ruma<create_room::Request>,
) -> MatrixResult<create_room::Response> {
    // TODO: check if room is unique
    let room_id = RoomId::new(data.hostname()).expect("host is valid");
    let user_id = body.user_id.clone().expect("user is authenticated");

    data.pdu_append(
        room_id.clone(),
        user_id.clone(),
        EventType::RoomCreate,
        json!({ "creator": user_id }),
        None,
        Some("".to_owned()),
    );

    if let Some(name) = &body.name {
        data.pdu_append(
            room_id.clone(),
            user_id.clone(),
            EventType::RoomName,
            json!({ "name": name }),
            None,
            Some("".to_owned()),
        );
    }

    if let Some(topic) = &body.topic {
        data.pdu_append(
            room_id.clone(),
            user_id.clone(),
            EventType::RoomTopic,
            json!({ "topic": topic }),
            None,
            Some("".to_owned()),
        );
    }

    data.room_join(&room_id, &user_id);

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
    if data.room_join(
        &body.room_id,
        body.user_id.as_ref().expect("user is authenticated"),
    ) {
        MatrixResult(Ok(join_room_by_id::Response {
            room_id: body.room_id.clone(),
        }))
    } else {
        MatrixResult(Err(Error {
            kind: ErrorKind::NotFound,
            message: "Room not found.".to_owned(),
            status_code: http::StatusCode::NOT_FOUND,
        }))
    }
}

#[post("/_matrix/client/r0/join/<_room_id_or_alias>", data = "<body>")]
fn join_room_by_id_or_alias_route(
    data: State<Data>,
    body: Ruma<join_room_by_id_or_alias::Request>,
    _room_id_or_alias: String,
) -> MatrixResult<join_room_by_id_or_alias::Response> {
    let room_id = match &body.room_id_or_alias {
        RoomIdOrAliasId::RoomAliasId(alias) => match alias.alias() {
            "#room:localhost" => "!xclkjvdlfj:localhost".try_into().unwrap(),
            _ => {
                debug!("Room not found.");
                return MatrixResult(Err(Error {
                    kind: ErrorKind::NotFound,
                    message: "Room not found.".to_owned(),
                    status_code: http::StatusCode::NOT_FOUND,
                }));
            }
        },

        RoomIdOrAliasId::RoomId(id) => id.clone(),
    };

    if data.room_join(
        &room_id,
        body.user_id.as_ref().expect("user is authenticated"),
    ) {
        MatrixResult(Ok(join_room_by_id_or_alias::Response { room_id }))
    } else {
        MatrixResult(Err(Error {
            kind: ErrorKind::NotFound,
            message: "Room not found.".to_owned(),
            status_code: http::StatusCode::NOT_FOUND,
        }))
    }
}

#[post("/_matrix/client/r0/publicRooms", data = "<body>")]
fn get_public_rooms_filtered_route(
    data: State<Data>,
    body: Ruma<get_public_rooms_filtered::Request>,
) -> MatrixResult<get_public_rooms_filtered::Response> {
    let chunk = data
        .rooms_all()
        .into_iter()
        .map(|room_id| directory::PublicRoomsChunk {
            aliases: None,
            canonical_alias: None,
            name: None,
            num_joined_members: data.room_users(&room_id).into(),
            room_id,
            topic: None,
            world_readable: false,
            guest_can_join: true,
            avatar_url: None,
        })
        .collect::<Vec<_>>();

    let total_room_count_estimate = (chunk.len() as u32).into();

    MatrixResult(Ok(get_public_rooms_filtered::Response {
        chunk,
        prev_batch: None,
        next_batch: None,
        total_room_count_estimate: Some(total_room_count_estimate),
    }))
}

#[get("/_matrix/client/r0/thirdparty/protocols", data = "<body>")]
fn get_protocols_route(
    body: Ruma<get_protocols::Request>,
) -> MatrixResult<get_protocols::Response> {
    MatrixResult(Ok(dbg!(get_protocols::Response {
        protocols: HashMap::new(),
    })))
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
    let mut unsigned = serde_json::Map::new();
    unsigned.insert("transaction_id".to_owned(), body.txn_id.clone().into());

    let event_id = data.pdu_append(
        body.room_id.clone(),
        body.user_id.clone().expect("user is authenticated"),
        body.event_type.clone(),
        body.json_body.clone(),
        Some(unsigned),
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
        None,
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
        None,
        Some("".to_owned()),
    );
    MatrixResult(Ok(create_state_event_for_empty_key::Response { event_id }))
}

#[get("/_matrix/client/r0/sync", data = "<body>")]
fn sync_route(
    data: State<Data>,
    body: Ruma<sync_events::Request>,
) -> MatrixResult<sync_events::Response> {
    std::thread::sleep(Duration::from_millis(200));
    let next_batch = data.last_pdu_index().to_string();

    let mut joined_rooms = HashMap::new();
    let joined_roomids = data.rooms_joined(body.user_id.as_ref().expect("user is authenticated"));
    for room_id in joined_roomids {
        let pdus = if let Some(since) = body.since.clone().and_then(|string| string.parse().ok()) {
            data.pdus_since(&room_id, since)
        } else {
            data.pdus_all(&room_id)
        };
        let room_events = pdus.into_iter().map(|pdu| pdu.to_room_event()).collect();

        joined_rooms.insert(
            room_id.try_into().unwrap(),
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
        next_batch,
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
        status_code: http::StatusCode::OK,
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
                get_login_route,
                login_route,
                get_pushrules_all_route,
                get_filter_route,
                create_filter_route,
                set_global_account_data_route,
                get_global_account_data_route,
                set_displayname_route,
                get_displayname_route,
                set_avatar_url_route,
                get_avatar_url_route,
                get_profile_route,
                set_presence_route,
                get_keys_route,
                upload_keys_route,
                create_room_route,
                get_alias_route,
                join_room_by_id_route,
                join_room_by_id_or_alias_route,
                get_public_rooms_filtered_route,
                get_protocols_route,
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
