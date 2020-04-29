use crate::{server_server, utils, Data, MatrixResult, Ruma};

use log::debug;
use rocket::{get, options, post, put, State};
use ruma_client_api::{
    error::{Error, ErrorKind},
    r0::{
        account::register,
        alias::get_alias,
        capabilities::get_capabilities,
        config::{get_global_account_data, set_global_account_data},
        directory::{self, get_public_rooms_filtered},
        filter::{self, create_filter, get_filter},
        keys::{get_keys, upload_keys},
        membership::{
            get_member_events, invite_user, join_room_by_id, join_room_by_id_or_alias, forget_room, leave_room,
        },
        message::{get_message_events, create_message_event},
        presence::set_presence,
        profile::{
            get_avatar_url, get_display_name, get_profile, set_avatar_url, set_display_name,
        },
        push::get_pushrules_all,
        read_marker::set_read_marker,
        room::create_room,
        session::{get_login_types, login},
        state::{create_state_event_for_empty_key, create_state_event_for_key},
        sync::sync_events,
        thirdparty::get_protocols,
        typing::create_typing_event,
        uiaa::{AuthFlow, UiaaInfo, UiaaResponse},
        user_directory::search_users,
    },
    unversioned::get_supported_versions,
};
use ruma_events::{collections::only::Event as EduEvent, EventType};
use ruma_identifiers::{RoomId, UserId};
use serde_json::json;
use std::{
    collections::BTreeMap,
    convert::{TryFrom, TryInto},
    path::PathBuf,
    time::{Duration, SystemTime},
};

const GUEST_NAME_LENGTH: usize = 10;
const DEVICE_ID_LENGTH: usize = 10;
const SESSION_ID_LENGTH: usize = 256;
const TOKEN_LENGTH: usize = 256;

#[get("/_matrix/client/versions")]
pub fn get_supported_versions_route() -> MatrixResult<get_supported_versions::Response> {
    MatrixResult(Ok(get_supported_versions::Response {
        versions: vec!["r0.5.0".to_owned(), "r0.6.0".to_owned()],
        unstable_features: BTreeMap::new(),
    }))
}

#[post("/_matrix/client/r0/register", data = "<body>")]
pub fn register_route(
    data: State<Data>,
    body: Ruma<register::Request>,
) -> MatrixResult<register::Response, UiaaResponse> {
    if body.auth.is_none() {
        return MatrixResult(Err(UiaaResponse::AuthResponse(UiaaInfo {
            flows: vec![AuthFlow {
                stages: vec!["m.login.dummy".to_owned()],
            }],
            completed: vec![],
            params: json!({}),
            session: Some(utils::random_string(SESSION_ID_LENGTH)),
            auth_error: None,
        })));
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
            return MatrixResult(Err(UiaaResponse::MatrixError(Error {
                kind: ErrorKind::InvalidUsername,
                message: "Username was invalid.".to_owned(),
                status_code: http::StatusCode::BAD_REQUEST,
            })));
        }
        Ok(user_id) => user_id,
    };

    // Check if username is creative enough
    if data.user_exists(&user_id) {
        debug!("ID already taken");
        return MatrixResult(Err(UiaaResponse::MatrixError(Error {
            kind: ErrorKind::UserInUse,
            message: "Desired user ID is already taken.".to_owned(),
            status_code: http::StatusCode::BAD_REQUEST,
        })));
    }

    let password = body.password.clone().unwrap_or_default();

    if let Ok(hash) = utils::calculate_hash(&password) {
        // Create user
        data.user_add(&user_id, &hash);
    } else {
        return MatrixResult(Err(UiaaResponse::MatrixError(Error {
            kind: ErrorKind::InvalidParam,
            message: "Password did not met requirements".to_owned(),
            status_code: http::StatusCode::BAD_REQUEST,
        })));
    }

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

#[get("/_matrix/client/r0/login")]
pub fn get_login_route() -> MatrixResult<get_login_types::Response> {
    MatrixResult(Ok(get_login_types::Response {
        flows: vec![get_login_types::LoginType::Password],
    }))
}

#[post("/_matrix/client/r0/login", data = "<body>")]
pub fn login_route(data: State<Data>, body: Ruma<login::Request>) -> MatrixResult<login::Response> {
    // Validate login method
    let user_id =
        if let (login::UserInfo::MatrixId(mut username), login::LoginInfo::Password { password }) =
            (body.user.clone(), body.login_info.clone())
        {
            if !username.contains(':') {
                username = format!("@{}:{}", username, data.hostname());
            }
            if let Ok(user_id) = (*username).try_into() {
                if let Some(hash) = data.password_hash_get(&user_id) {
                    let hash_matches =
                        argon2::verify_encoded(&hash, password.as_bytes()).unwrap_or(false);

                    if hash_matches {
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

#[get("/_matrix/client/r0/capabilities", data = "<body>")]
pub fn get_capabilities_route(
    body: Ruma<get_capabilities::Request>,
) -> MatrixResult<get_capabilities::Response> {
    // TODO
    //let mut available = BTreeMap::new();
    //available.insert("5".to_owned(), get_capabilities::RoomVersionStability::Unstable);

    MatrixResult(Ok(get_capabilities::Response {
        capabilities: get_capabilities::Capabilities {
            change_password: None,
            room_versions: None, //Some(get_capabilities::RoomVersionsCapability { default: "5".to_owned(), available }),
            custom_capabilities: BTreeMap::new(),
        },
    }))
}

#[get("/_matrix/client/r0/pushrules")]
pub fn get_pushrules_all_route() -> MatrixResult<get_pushrules_all::Response> {
    // TODO
    MatrixResult(Ok(get_pushrules_all::Response {
        global: BTreeMap::new(),
    }))
}

#[get(
    "/_matrix/client/r0/user/<_user_id>/filter/<_filter_id>",
    data = "<body>"
)]
pub fn get_filter_route(
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
pub fn create_filter_route(
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
pub fn set_global_account_data_route(
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
pub fn get_global_account_data_route(
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
pub fn set_displayname_route(
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

    if let Some(displayname) = &body.displayname {
        // Some("") will clear the displayname
        if displayname == "" {
            data.displayname_remove(&user_id);
        } else {
            data.displayname_set(&user_id, displayname.clone());
            // TODO send a new m.presence event with the updated displayname
        }
    }

    MatrixResult(Ok(set_display_name::Response))
}

#[get(
    "/_matrix/client/r0/profile/<user_id_raw>/displayname",
    data = "<body>"
)]
pub fn get_displayname_route(
    data: State<Data>,
    body: Ruma<get_display_name::Request>,
    user_id_raw: String,
) -> MatrixResult<get_display_name::Response> {
    let user_id = (*body).user_id.clone();
    if !data.user_exists(&user_id) {
        // Return 404 if we don't have a profile for this id
        debug!("Profile was not found.");
        return MatrixResult(Err(Error {
            kind: ErrorKind::NotFound,
            message: "Profile was not found.".to_owned(),
            status_code: http::StatusCode::NOT_FOUND,
        }));
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
pub fn set_avatar_url_route(
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
        data.avatar_url_set(&user_id, body.avatar_url.clone());
        // TODO send a new m.room.member join event with the updated avatar_url
        // TODO send a new m.presence event with the updated avatar_url
    }

    MatrixResult(Ok(set_avatar_url::Response))
}

#[get("/_matrix/client/r0/profile/<user_id_raw>/avatar_url", data = "<body>")]
pub fn get_avatar_url_route(
    data: State<Data>,
    body: Ruma<get_avatar_url::Request>,
    user_id_raw: String,
) -> MatrixResult<get_avatar_url::Response> {
    let user_id = (*body).user_id.clone();
    if !data.user_exists(&user_id) {
        // Return 404 if we don't have a profile for this id
        debug!("Profile was not found.");
        return MatrixResult(Err(Error {
            kind: ErrorKind::NotFound,
            message: "Profile was not found.".to_owned(),
            status_code: http::StatusCode::NOT_FOUND,
        }));
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
pub fn get_profile_route(
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
pub fn set_presence_route(
    body: Ruma<set_presence::Request>,
    _user_id: String,
) -> MatrixResult<set_presence::Response> {
    // TODO
    MatrixResult(Ok(set_presence::Response))
}

#[post("/_matrix/client/r0/keys/query", data = "<body>")]
pub fn get_keys_route(body: Ruma<get_keys::Request>) -> MatrixResult<get_keys::Response> {
    // TODO
    MatrixResult(Ok(get_keys::Response {
        failures: BTreeMap::new(),
        device_keys: BTreeMap::new(),
    }))
}

#[post("/_matrix/client/r0/keys/upload", data = "<body>")]
pub fn upload_keys_route(
    data: State<Data>,
    body: Ruma<upload_keys::Request>,
) -> MatrixResult<upload_keys::Response> {
    // TODO
    MatrixResult(Ok(upload_keys::Response {
        one_time_key_counts: BTreeMap::new(),
    }))
}

#[post("/_matrix/client/r0/rooms/<_room_id>/read_markers", data = "<body>")]
pub fn set_read_marker_route(
    data: State<Data>,
    body: Ruma<set_read_marker::Request>,
    _room_id: String,
) -> MatrixResult<set_read_marker::Response> {
    let user_id = body.user_id.clone().expect("user is authenticated");
    // TODO: Fully read
    if let Some(event) = &body.read_receipt {
        let mut user_receipts = BTreeMap::new();
        user_receipts.insert(
            user_id.clone(),
            ruma_events::receipt::Receipt {
                ts: Some(SystemTime::now()),
            },
        );
        let mut receipt_content = BTreeMap::new();
        receipt_content.insert(
            event.clone(),
            ruma_events::receipt::Receipts {
                read: Some(user_receipts),
            },
        );

        data.roomlatest_update(
            &user_id,
            &body.room_id,
            EduEvent::Receipt(ruma_events::receipt::ReceiptEvent {
                content: receipt_content,
                room_id: None, // None because it can be inferred
            }),
        );
    }
    MatrixResult(Ok(set_read_marker::Response))
}

#[put(
    "/_matrix/client/r0/rooms/<_room_id>/typing/<_user_id>",
    data = "<body>"
)]
pub fn create_typing_event_route(
    data: State<Data>,
    body: Ruma<create_typing_event::Request>,
    _room_id: String,
    _user_id: String,
) -> MatrixResult<create_typing_event::Response> {
    let user_id = body.user_id.clone().expect("user is authenticated");
    let edu = EduEvent::Typing(ruma_events::typing::TypingEvent {
        content: ruma_events::typing::TypingEventContent {
            user_ids: vec![user_id.clone()],
        },
        room_id: None, // None because it can be inferred
    });

    if body.typing {
        data.roomactive_add(
            edu,
            &body.room_id,
            body.timeout.map(|d| d.as_millis() as u64).unwrap_or(30000)
                + utils::millis_since_unix_epoch().try_into().unwrap_or(0),
        );
    } else {
        data.roomactive_remove(edu, &body.room_id);
    }

    MatrixResult(Ok(create_typing_event::Response))
}

#[post("/_matrix/client/r0/createRoom", data = "<body>")]
pub fn create_room_route(
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

    data.pdu_append(
        room_id.clone(),
        user_id.clone(),
        EventType::RoomPowerLevels,
        json!({
            "ban": 50,
            "events_default": 0,
            "invite": 50,
            "kick": 50,
            "redact": 50,
            "state_default": 50,
            "users": { user_id.to_string(): 100 },
            "users_default": 0
        }),
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

    for user in &body.invite {
        data.room_invite(&user_id, &room_id, user);
    }

    MatrixResult(Ok(create_room::Response { room_id }))
}

#[get("/_matrix/client/r0/directory/room/<_room_alias>", data = "<body>")]
pub fn get_alias_route(
    data: State<Data>,
    body: Ruma<get_alias::Request>,
    _room_alias: String,
) -> MatrixResult<get_alias::Response> {
    // TODO
    let room_id = if body.room_alias.server_name() == data.hostname() {
        match body.room_alias.alias() {
            "conduit" => "!lgOCCXQKtXOAPlAlG5:conduit.rs",
            _ => {
                debug!("Room alias not found.");
                return MatrixResult(Err(Error {
                    kind: ErrorKind::NotFound,
                    message: "Room not found.".to_owned(),
                    status_code: http::StatusCode::NOT_FOUND,
                }));
            }
        }
    } else {
        todo!("ask remote server");
    }
    .try_into()
    .unwrap();

    MatrixResult(Ok(get_alias::Response {
        room_id,
        servers: vec!["conduit.rs".to_owned()],
    }))
}

#[post("/_matrix/client/r0/rooms/<_room_id>/join", data = "<body>")]
pub fn join_room_by_id_route(
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
pub fn join_room_by_id_or_alias_route(
    data: State<Data>,
    body: Ruma<join_room_by_id_or_alias::Request>,
    _room_id_or_alias: String,
) -> MatrixResult<join_room_by_id_or_alias::Response> {
    let room_id = match RoomId::try_from(body.room_id_or_alias.clone()) {
        Ok(room_id) => room_id,
        Err(room_alias) => if room_alias.server_name() == data.hostname() {
            return MatrixResult(Err(Error {
                kind: ErrorKind::NotFound,
                message: "Room alias not found.".to_owned(),
                status_code: http::StatusCode::NOT_FOUND,
            }));
        } else {
            // Ask creator server of the room to join TODO ask someone else when not available
            //server_server::send_request(data, destination, request)
            todo!();
        }
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

#[post("/_matrix/client/r0/rooms/<_room_id>/leave", data = "<body>")]
pub fn leave_room_route(
    data: State<Data>,
    body: Ruma<leave_room::Request>,
    _room_id: String,
) -> MatrixResult<leave_room::Response> {
    let user_id = body.user_id.clone().expect("user is authenticated");
    data.room_leave(&user_id, &body.room_id, &user_id);
    MatrixResult(Ok(leave_room::Response))
}

#[post("/_matrix/client/r0/rooms/<_room_id>/forget", data = "<body>")]
pub fn forget_room_route(
    data: State<Data>,
    body: Ruma<forget_room::Request>,
    _room_id: String,
) -> MatrixResult<forget_room::Response> {
    let user_id = body.user_id.clone().expect("user is authenticated");
    data.room_forget(&body.room_id, &user_id);
    MatrixResult(Ok(forget_room::Response))
}

#[post("/_matrix/client/r0/rooms/<_room_id>/invite", data = "<body>")]
pub fn invite_user_route(
    data: State<Data>,
    body: Ruma<invite_user::Request>,
    _room_id: String,
) -> MatrixResult<invite_user::Response> {
    if let invite_user::InvitationRecipient::UserId { user_id } = &body.recipient {
        data.room_invite(
            &body.user_id.as_ref().expect("user is authenticated"),
            &body.room_id,
            &user_id,
        );
        MatrixResult(Ok(invite_user::Response))
    } else {
        MatrixResult(Err(Error {
            kind: ErrorKind::NotFound,
            message: "User not found.".to_owned(),
            status_code: http::StatusCode::NOT_FOUND,
        }))
    }
}

#[post("/_matrix/client/r0/publicRooms", data = "<body>")]
pub async fn get_public_rooms_filtered_route(
    data: State<'_, Data>,
    body: Ruma<get_public_rooms_filtered::Request>,
) -> MatrixResult<get_public_rooms_filtered::Response> {
    let mut chunk = data
        .rooms_all()
        .into_iter()
        .map(|room_id| {
            let state = data.room_state(&room_id);
            directory::PublicRoomsChunk {
                aliases: Vec::new(),
                canonical_alias: None,
                name: state
                    .get(&(EventType::RoomName, "".to_owned()))
                    .and_then(|s| s.content.get("name"))
                    .and_then(|n| n.as_str())
                    .map(|n| n.to_owned()),
                num_joined_members: data.room_users(&room_id).into(),
                room_id,
                topic: None,
                world_readable: false,
                guest_can_join: true,
                avatar_url: None,
            }
        })
        .collect::<Vec<_>>();

    chunk.sort_by(|l, r| r.num_joined_members.cmp(&l.num_joined_members));

    chunk.extend_from_slice(
        &server_server::send_request(
            &data,
            "privacytools.io".to_owned(),
            ruma_federation_api::v1::get_public_rooms::Request {
                limit: Some(20_u32.into()),
                since: None,
                room_network: ruma_federation_api::v1::get_public_rooms::RoomNetwork::Matrix,
            },
        )
        .await
        .unwrap()
        .chunk
        .into_iter()
        .map(|c| serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap())
        .collect::<Vec<_>>(),
    );

    let total_room_count_estimate = (chunk.len() as u32).into();

    MatrixResult(Ok(get_public_rooms_filtered::Response {
        chunk,
        prev_batch: None,
        next_batch: None,
        total_room_count_estimate: Some(total_room_count_estimate),
    }))
}

#[post("/_matrix/client/r0/user_directory/search", data = "<body>")]
pub fn search_users_route(
    data: State<Data>,
    body: Ruma<search_users::Request>,
) -> MatrixResult<search_users::Response> {
    MatrixResult(Ok(search_users::Response {
        results: data
            .users_all()
            .into_iter()
            .filter(|user_id| user_id.to_string().contains(&body.search_term))
            .map(|user_id| search_users::User {
                user_id,
                display_name: None,
                avatar_url: None,
            })
            .collect(),
        limited: false,
    }))
}

#[get("/_matrix/client/r0/rooms/<_room_id>/members", data = "<body>")]
pub fn get_member_events_route(
    body: Ruma<get_member_events::Request>,
    _room_id: String,
) -> MatrixResult<get_member_events::Response> {
    // TODO
    MatrixResult(Ok(get_member_events::Response { chunk: Vec::new() }))
}

#[get("/_matrix/client/r0/thirdparty/protocols", data = "<body>")]
pub fn get_protocols_route(
    body: Ruma<get_protocols::Request>,
) -> MatrixResult<get_protocols::Response> {
    // TODO
    MatrixResult(Ok(get_protocols::Response {
        protocols: BTreeMap::new(),
    }))
}

#[put(
    "/_matrix/client/r0/rooms/<_room_id>/send/<_event_type>/<_txn_id>",
    data = "<body>"
)]
pub fn create_message_event_route(
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
pub fn create_state_event_for_key_route(
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
pub fn create_state_event_for_empty_key_route(
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
pub fn sync_route(
    data: State<Data>,
    body: Ruma<sync_events::Request>,
) -> MatrixResult<sync_events::Response> {
    std::thread::sleep(Duration::from_millis(300));
    let next_batch = data.last_pdu_index().to_string();

    let mut joined_rooms = BTreeMap::new();
    let joined_roomids = data.rooms_joined(body.user_id.as_ref().expect("user is authenticated"));
    let since = body
        .since
        .clone()
        .and_then(|string| string.parse().ok())
        .unwrap_or(0);
    for room_id in joined_roomids {
        let pdus = data.pdus_since(&room_id, since);
        let room_events = pdus.into_iter().map(|pdu| pdu.to_room_event()).collect::<Vec<_>>();
        let is_first_pdu = data.room_pdu_first(&room_id, since);
        let mut edus = data.roomlatests_since(&room_id, since);
        edus.extend_from_slice(&data.roomactives_in(&room_id));

        joined_rooms.insert(
            room_id.clone().try_into().unwrap(),
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
                    limited: None,
                    prev_batch: Some(since.to_string()),
                    events: room_events,
                },
                state: sync_events::State { events: Vec::new() },
                ephemeral: sync_events::Ephemeral { events: edus },
            },
        );
    }

    let mut left_rooms = BTreeMap::new();
    let left_roomids = data.rooms_left(body.user_id.as_ref().expect("user is authenticated"));
    for room_id in left_roomids {
        let pdus = data.pdus_since(&room_id, since);
        let room_events = pdus.into_iter().map(|pdu| pdu.to_room_event()).collect();
        let mut edus = data.roomlatests_since(&room_id, since);
        edus.extend_from_slice(&data.roomactives_in(&room_id));

        left_rooms.insert(
            room_id.clone().try_into().unwrap(),
            sync_events::LeftRoom {
                account_data: sync_events::AccountData { events: Vec::new() },
                timeline: sync_events::Timeline {
                    limited: Some(false),
                    prev_batch: Some(next_batch.clone()),
                    events: room_events,
                },
                state: sync_events::State { events: Vec::new() },
            },
        );
    }

    let mut invited_rooms = BTreeMap::new();
    for room_id in data.rooms_invited(body.user_id.as_ref().expect("user is authenticated")) {
        let events = data
            .pdus_since(&room_id, since)
            .into_iter()
            .map(|pdu| pdu.to_stripped_state_event())
            .collect();

        invited_rooms.insert(
            room_id,
            sync_events::InvitedRoom {
                invite_state: sync_events::InviteState { events },
            },
        );
    }

    MatrixResult(Ok(sync_events::Response {
        next_batch,
        rooms: sync_events::Rooms {
            leave: left_rooms,
            join: joined_rooms,
            invite: invited_rooms,
        },
        presence: sync_events::Presence { events: Vec::new() },
        device_lists: Default::default(),
        device_one_time_keys_count: Default::default(),
        to_device: sync_events::ToDevice { events: Vec::new() },
    }))
}

#[get("/_matrix/client/r0/rooms/<_room_id>/messages", data = "<body>")]
pub fn get_message_events_route(
    data: State<Data>,
    body: Ruma<get_message_events::Request>,
    _room_id: String) -> MatrixResult<get_message_events::Response> {
    if let get_message_events::Direction::Forward = body.dir {todo!();}

    if let Ok(from) = body
        .from
        .clone()
        .parse() {
        let pdus = data.pdus_until(&body.room_id, from);
        let room_events = pdus.into_iter().map(|pdu| pdu.to_room_event()).collect::<Vec<_>>();
        MatrixResult(Ok(get_message_events::Response {
            start: Some(body.from.clone()),
            end: None,
            chunk: room_events,
            state: Vec::new(),
        }))
    } else {
        MatrixResult(Err(Error {
            kind: ErrorKind::NotFound,
            message: "Invalid from.".to_owned(),
            status_code: http::StatusCode::BAD_REQUEST,
        }))
    }
}

#[get("/_matrix/client/r0/voip/turnServer")]
pub fn turn_server_route() -> MatrixResult<create_message_event::Response> {
    // TODO
    MatrixResult(Err(Error {
        kind: ErrorKind::NotFound,
        message: "There is no turn server yet.".to_owned(),
        status_code: http::StatusCode::NOT_FOUND,
    }))
}

#[post("/_matrix/client/r0/publicised_groups")]
pub fn publicised_groups_route() -> MatrixResult<create_message_event::Response> {
    // TODO
    MatrixResult(Err(Error {
        kind: ErrorKind::NotFound,
        message: "There are no publicised groups yet.".to_owned(),
        status_code: http::StatusCode::NOT_FOUND,
    }))
}

#[options("/<_segments..>")]
pub fn options_route(_segments: rocket::http::uri::Segments) -> MatrixResult<create_message_event::Response> {
    MatrixResult(Err(Error {
        kind: ErrorKind::NotFound,
        message: "This is the options route.".to_owned(),
        status_code: http::StatusCode::OK,
    }))
}
