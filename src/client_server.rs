use std::{
    collections::BTreeMap,
    convert::{TryFrom, TryInto},
    time::{Duration, SystemTime},
};

use log::{debug, warn};
use rocket::{get, options, post, put, State};
use ruma_client_api::{
    error::{Error, ErrorKind},
    r0::{
        account::{get_username_availability, register},
        alias::get_alias,
        capabilities::get_capabilities,
        config::{get_global_account_data, set_global_account_data},
        directory::{self, get_public_rooms_filtered},
        filter::{self, create_filter, get_filter},
        keys::{claim_keys, get_keys, upload_keys},
        media::get_media_config,
        membership::{
            forget_room, get_member_events, invite_user, join_room_by_id, join_room_by_id_or_alias,
            leave_room,
        },
        message::{create_message_event, get_message_events},
        presence::set_presence,
        profile::{
            get_avatar_url, get_display_name, get_profile, set_avatar_url, set_display_name,
        },
        push::{self, get_pushrules_all, set_pushrule, set_pushrule_enabled},
        read_marker::set_read_marker,
        room::create_room,
        session::{get_login_types, login},
        state::{
            create_state_event_for_empty_key, create_state_event_for_key, get_state_events,
            get_state_events_for_empty_key, get_state_events_for_key,
        },
        sync::sync_events,
        thirdparty::get_protocols,
        to_device::{self, send_event_to_device},
        typing::create_typing_event,
        uiaa::{AuthFlow, UiaaInfo, UiaaResponse},
        user_directory::search_users,
    },
    unversioned::get_supported_versions,
};
use ruma_events::{collections::only::Event as EduEvent, EventJson, EventType};
use ruma_identifiers::{RoomId, UserId};
use serde_json::{json, value::RawValue};

use crate::{server_server, utils, Database, MatrixResult, Ruma};

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

#[get("/_matrix/client/r0/register/available", data = "<body>")]
pub fn get_register_available_route(
    db: State<'_, Database>,
    body: Ruma<get_username_availability::Request>,
) -> MatrixResult<get_username_availability::Response> {
    // Validate user id
    let user_id =
        match UserId::parse_with_server_name(body.username.clone(), db.globals.server_name())
            .ok()
            .filter(|user_id| !user_id.is_historical())
        {
            None => {
                debug!("Username invalid");
                return MatrixResult(Err(Error {
                    kind: ErrorKind::InvalidUsername,
                    message: "Username was invalid.".to_owned(),
                    status_code: http::StatusCode::BAD_REQUEST,
                }));
            }
            Some(user_id) => user_id,
        };

    // Check if username is creative enough
    if db.users.exists(&user_id).unwrap() {
        debug!("ID already taken");
        return MatrixResult(Err(Error {
            kind: ErrorKind::UserInUse,
            message: "Desired user ID is already taken.".to_owned(),
            status_code: http::StatusCode::BAD_REQUEST,
        }));
    }

    // TODO add check for appservice namespaces

    // If no if check is true we have an username that's available to be used.
    MatrixResult(Ok(get_username_availability::Response { available: true }))
}

#[post("/_matrix/client/r0/register", data = "<body>")]
pub fn register_route(
    db: State<'_, Database>,
    body: Ruma<register::Request>,
) -> MatrixResult<register::Response, UiaaResponse> {
    if body.auth.is_none() {
        return MatrixResult(Err(UiaaResponse::AuthResponse(UiaaInfo {
            flows: vec![AuthFlow {
                stages: vec!["m.login.dummy".to_owned()],
            }],
            completed: vec![],
            params: RawValue::from_string("{}".to_owned()).unwrap(),
            session: Some(utils::random_string(SESSION_ID_LENGTH)),
            auth_error: None,
        })));
    }

    // Validate user id
    let user_id = match UserId::parse_with_server_name(
        body.username
            .clone()
            .unwrap_or_else(|| utils::random_string(GUEST_NAME_LENGTH)),
        db.globals.server_name(),
    )
    .ok()
    .filter(|user_id| !user_id.is_historical())
    {
        None => {
            debug!("Username invalid");
            return MatrixResult(Err(UiaaResponse::MatrixError(Error {
                kind: ErrorKind::InvalidUsername,
                message: "Username was invalid.".to_owned(),
                status_code: http::StatusCode::BAD_REQUEST,
            })));
        }
        Some(user_id) => user_id,
    };

    // Check if username is creative enough
    if db.users.exists(&user_id).unwrap() {
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
        db.users.create(&user_id, &hash).unwrap();
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
        .unwrap_or_else(|| utils::random_string(DEVICE_ID_LENGTH));

    // Generate new token for the device
    let token = utils::random_string(TOKEN_LENGTH);

    // Add device
    db.users
        .create_device(&user_id, &device_id, &token)
        .unwrap();

    // Initial data
    db.account_data
        .update(
            None,
            &user_id,
            &EventType::PushRules,
            serde_json::to_value(ruma_events::push_rules::PushRulesEvent {
                content: ruma_events::push_rules::PushRulesEventContent {
                    global: ruma_events::push_rules::Ruleset {
                        content: vec![],
                        override_rules: vec![],
                        room: vec![],
                        sender: vec![],
                        underride: vec![ruma_events::push_rules::ConditionalPushRule {
                            actions: vec![
                                ruma_events::push_rules::Action::Notify,
                                ruma_events::push_rules::Action::SetTweak(
                                    ruma_common::push::Tweak::Highlight(false),
                                ),
                            ],
                            default: true,
                            enabled: true,
                            rule_id: ".m.rule.message".to_owned(),
                            conditions: vec![ruma_events::push_rules::PushCondition::EventMatch(
                                ruma_events::push_rules::EventMatchCondition {
                                    key: "type".to_owned(),
                                    pattern: "m.room.message".to_owned(),
                                },
                            )],
                        }],
                    },
                },
            })
            .unwrap()
            .as_object_mut()
            .unwrap(),
            &db.globals,
        )
        .unwrap();

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
pub fn login_route(
    db: State<'_, Database>,
    body: Ruma<login::Request>,
) -> MatrixResult<login::Response> {
    // Validate login method
    let user_id =
        if let (login::UserInfo::MatrixId(mut username), login::LoginInfo::Password { password }) =
            (body.user.clone(), body.login_info.clone())
        {
            if !username.contains(':') {
                username = format!("@{}:{}", username, db.globals.server_name());
            }
            if let Ok(user_id) = (*username).try_into() {
                if let Some(hash) = db.users.password_hash(&user_id).unwrap() {
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
        .unwrap_or_else(|| utils::random_string(DEVICE_ID_LENGTH));

    // Generate a new token for the device
    let token = utils::random_string(TOKEN_LENGTH);

    // Add device
    db.users
        .create_device(&user_id, &device_id, &token)
        .unwrap();

    MatrixResult(Ok(login::Response {
        user_id,
        access_token: token,
        home_server: Some(db.globals.server_name().to_owned()),
        device_id,
        well_known: None,
    }))
}

#[get("/_matrix/client/r0/capabilities")]
pub fn get_capabilities_route() -> MatrixResult<get_capabilities::Response> {
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
    let mut global = BTreeMap::new();
    global.insert(
        push::RuleKind::Underride,
        vec![push::PushRule {
            actions: vec![
                push::Action::Notify,
                push::Action::SetTweak(ruma_common::push::Tweak::Highlight(false)),
            ],
            default: true,
            enabled: true,
            rule_id: ".m.rule.message".to_owned(),
            conditions: Some(vec![push::PushCondition::EventMatch {
                key: "type".to_owned(),
                pattern: "m.room.message".to_owned(),
            }]),
            pattern: None,
        }],
    );
    MatrixResult(Ok(get_pushrules_all::Response { global }))
}

#[put(
    "/_matrix/client/r0/pushrules/<_scope>/<_kind>/<_rule_id>",
    data = "<body>"
)]
pub fn set_pushrule_route(
    db: State<'_, Database>,
    body: Ruma<set_pushrule::Request>,
    _scope: String,
    _kind: String,
    _rule_id: String,
) -> MatrixResult<set_pushrule::Response> {
    // TODO
    let user_id = body.user_id.as_ref().expect("user is authenticated");
    db.account_data
        .update(
            None,
            &user_id,
            &EventType::PushRules,
            serde_json::to_value(ruma_events::push_rules::PushRulesEvent {
                content: ruma_events::push_rules::PushRulesEventContent {
                    global: ruma_events::push_rules::Ruleset {
                        content: vec![],
                        override_rules: vec![],
                        room: vec![],
                        sender: vec![],
                        underride: vec![ruma_events::push_rules::ConditionalPushRule {
                            actions: vec![
                                ruma_events::push_rules::Action::Notify,
                                ruma_events::push_rules::Action::SetTweak(
                                    ruma_common::push::Tweak::Highlight(false),
                                ),
                            ],
                            default: true,
                            enabled: true,
                            rule_id: ".m.rule.message".to_owned(),
                            conditions: vec![ruma_events::push_rules::PushCondition::EventMatch(
                                ruma_events::push_rules::EventMatchCondition {
                                    key: "type".to_owned(),
                                    pattern: "m.room.message".to_owned(),
                                },
                            )],
                        }],
                    },
                },
            })
            .unwrap()
            .as_object_mut()
            .unwrap(),
            &db.globals,
        )
        .unwrap();

    MatrixResult(Ok(set_pushrule::Response))
}

#[put("/_matrix/client/r0/pushrules/<_scope>/<_kind>/<_rule_id>/enabled")]
pub fn set_pushrule_enabled_route(
    _scope: String,
    _kind: String,
    _rule_id: String,
) -> MatrixResult<set_pushrule_enabled::Response> {
    // TODO
    MatrixResult(Ok(set_pushrule_enabled::Response))
}

#[get("/_matrix/client/r0/user/<_user_id>/filter/<_filter_id>")]
pub fn get_filter_route(
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

#[post("/_matrix/client/r0/user/<_user_id>/filter")]
pub fn create_filter_route(_user_id: String) -> MatrixResult<create_filter::Response> {
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
    db: State<'_, Database>,
    body: Ruma<set_global_account_data::Request>,
    _user_id: String,
    _type: String,
) -> MatrixResult<set_global_account_data::Response> {
    let user_id = body.user_id.as_ref().expect("user is authenticated");

    db.account_data
        .update(
            None,
            user_id,
            &EventType::try_from(&body.event_type).unwrap(),
            json!({"content": serde_json::from_str::<serde_json::Value>(body.data.get()).unwrap()})
                .as_object_mut()
                .unwrap(),
            &db.globals,
        )
        .unwrap();

    MatrixResult(Ok(set_global_account_data::Response))
}

#[get(
    "/_matrix/client/r0/user/<_user_id>/account_data/<_type>",
    data = "<body>"
)]
pub fn get_global_account_data_route(
    db: State<'_, Database>,
    body: Ruma<get_global_account_data::Request>,
    _user_id: String,
    _type: String,
) -> MatrixResult<get_global_account_data::Response> {
    let user_id = body.user_id.as_ref().expect("user is authenticated");

    if let Some(data) = db
        .account_data
        .get(
            None,
            user_id,
            &EventType::try_from(&body.event_type).unwrap(),
        )
        .unwrap()
    {
        MatrixResult(Ok(get_global_account_data::Response { account_data: data }))
    } else {
        MatrixResult(Err(Error {
            kind: ErrorKind::NotFound,
            message: "Data not found.".to_owned(),
            status_code: http::StatusCode::NOT_FOUND,
        }))
    }
}

#[put("/_matrix/client/r0/profile/<_user_id>/displayname", data = "<body>")]
pub fn set_displayname_route(
    db: State<'_, Database>,
    body: Ruma<set_display_name::Request>,
    _user_id: String,
) -> MatrixResult<set_display_name::Response> {
    let user_id = body.user_id.as_ref().expect("user is authenticated");

    if let Some(displayname) = &body.displayname {
        // Some("") will clear the displayname
        if displayname == "" {
            db.users.set_displayname(&user_id, None).unwrap();
        } else {
            db.users
                .set_displayname(&user_id, Some(displayname.clone()))
                .unwrap();
        }

        // Send a new membership event into all joined rooms
        for room_id in db.rooms.rooms_joined(&user_id) {
            db.rooms
                .append_pdu(
                    room_id.unwrap(),
                    user_id.clone(),
                    EventType::RoomMember,
                    json!({"membership": "join", "displayname": displayname}),
                    None,
                    Some(user_id.to_string()),
                    &db.globals,
                )
                .unwrap();
        }

        // Presence update
        db.global_edus
            .update_globallatest(
                &user_id,
                EduEvent::Presence(ruma_events::presence::PresenceEvent {
                    content: ruma_events::presence::PresenceEventContent {
                        avatar_url: db.users.avatar_url(&user_id).unwrap(),
                        currently_active: None,
                        displayname: db.users.displayname(&user_id).unwrap(),
                        last_active_ago: Some(utils::millis_since_unix_epoch().try_into().unwrap()),
                        presence: ruma_events::presence::PresenceState::Online,
                        status_msg: None,
                    },
                    sender: user_id.clone(),
                }),
                &db.globals,
            )
            .unwrap();
    } else {
        // Send error on None
        // Synapse returns a parsing error but the spec doesn't require this
        debug!("Request was missing the displayname payload.");
        return MatrixResult(Err(Error {
            kind: ErrorKind::MissingParam,
            message: "Missing displayname.".to_owned(),
            status_code: http::StatusCode::BAD_REQUEST,
        }));
    }

    MatrixResult(Ok(set_display_name::Response))
}

#[get("/_matrix/client/r0/profile/<_user_id>/displayname", data = "<body>")]
pub fn get_displayname_route(
    db: State<'_, Database>,
    body: Ruma<get_display_name::Request>,
    _user_id: String,
) -> MatrixResult<get_display_name::Response> {
    let user_id = (*body).user_id.clone();
    if !db.users.exists(&user_id).unwrap() {
        // Return 404 if we don't have a profile for this id
        debug!("Profile was not found.");
        return MatrixResult(Err(Error {
            kind: ErrorKind::NotFound,
            message: "Profile was not found.".to_owned(),
            status_code: http::StatusCode::NOT_FOUND,
        }));
    }
    if let Some(displayname) = db.users.displayname(&user_id).unwrap() {
        return MatrixResult(Ok(get_display_name::Response {
            displayname: Some(displayname),
        }));
    }

    // The user has no displayname
    MatrixResult(Ok(get_display_name::Response { displayname: None }))
}

#[put("/_matrix/client/r0/profile/<_user_id>/avatar_url", data = "<body>")]
pub fn set_avatar_url_route(
    db: State<'_, Database>,
    body: Ruma<set_avatar_url::Request>,
    _user_id: String,
) -> MatrixResult<set_avatar_url::Response> {
    let user_id = body.user_id.as_ref().expect("user is authenticated");

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
        db.users.set_avatar_url(&user_id, None).unwrap();
    } else {
        db.users
            .set_avatar_url(&user_id, Some(body.avatar_url.clone()))
            .unwrap();
        // TODO send a new m.room.member join event with the updated avatar_url
        // TODO send a new m.presence event with the updated avatar_url
    }

    MatrixResult(Ok(set_avatar_url::Response))
}

#[get("/_matrix/client/r0/profile/<_user_id>/avatar_url", data = "<body>")]
pub fn get_avatar_url_route(
    db: State<'_, Database>,
    body: Ruma<get_avatar_url::Request>,
    _user_id: String,
) -> MatrixResult<get_avatar_url::Response> {
    let user_id = (*body).user_id.clone();
    if !db.users.exists(&user_id).unwrap() {
        // Return 404 if we don't have a profile for this id
        debug!("Profile was not found.");
        return MatrixResult(Err(Error {
            kind: ErrorKind::NotFound,
            message: "Profile was not found.".to_owned(),
            status_code: http::StatusCode::NOT_FOUND,
        }));
    }
    if let Some(avatar_url) = db.users.avatar_url(&user_id).unwrap() {
        return MatrixResult(Ok(get_avatar_url::Response {
            avatar_url: Some(avatar_url),
        }));
    }

    // The user has no avatar
    MatrixResult(Ok(get_avatar_url::Response { avatar_url: None }))
}

#[get("/_matrix/client/r0/profile/<_user_id>", data = "<body>")]
pub fn get_profile_route(
    db: State<'_, Database>,
    body: Ruma<get_profile::Request>,
    _user_id: String,
) -> MatrixResult<get_profile::Response> {
    let user_id = (*body).user_id.clone();
    let avatar_url = db.users.avatar_url(&user_id).unwrap();
    let displayname = db.users.displayname(&user_id).unwrap();

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
    db: State<'_, Database>,
    body: Ruma<set_presence::Request>,
    _user_id: String,
) -> MatrixResult<set_presence::Response> {
    let user_id = body.user_id.as_ref().expect("user is authenticated");

    db.global_edus
        .update_globallatest(
            &user_id,
            EduEvent::Presence(ruma_events::presence::PresenceEvent {
                content: ruma_events::presence::PresenceEventContent {
                    avatar_url: db.users.avatar_url(&user_id).unwrap(),
                    currently_active: None,
                    displayname: db.users.displayname(&user_id).unwrap(),
                    last_active_ago: Some(utils::millis_since_unix_epoch().try_into().unwrap()),
                    presence: body.presence,
                    status_msg: body.status_msg.clone(),
                },
                sender: user_id.clone(),
            }),
            &db.globals,
        )
        .unwrap();

    MatrixResult(Ok(set_presence::Response))
}

#[post("/_matrix/client/r0/keys/upload", data = "<body>")]
pub fn upload_keys_route(
    db: State<'_, Database>,
    body: Ruma<upload_keys::Request>,
) -> MatrixResult<upload_keys::Response> {
    let user_id = body.user_id.as_ref().expect("user is authenticated");
    let device_id = body.device_id.as_ref().expect("user is authenticated");

    if let Some(one_time_keys) = &body.one_time_keys {
        for (key_key, key_value) in one_time_keys {
            db.users
                .add_one_time_key(user_id, device_id, key_key, key_value)
                .unwrap();
        }
    }

    if let Some(device_keys) = &body.device_keys {
        db.users
            .add_device_keys(user_id, device_id, device_keys)
            .unwrap();
    }

    MatrixResult(Ok(upload_keys::Response {
        one_time_key_counts: db.users.count_one_time_keys(user_id, device_id).unwrap(),
    }))
}

#[post("/_matrix/client/r0/keys/query", data = "<body>")]
pub fn get_keys_route(
    db: State<'_, Database>,
    body: Ruma<get_keys::Request>,
) -> MatrixResult<get_keys::Response> {
    let mut device_keys = BTreeMap::new();

    for (user_id, device_ids) in &body.device_keys {
        if device_ids.is_empty() {
            let mut container = BTreeMap::new();
            for (device_id, keys) in db
                .users
                .all_device_keys(&user_id.clone())
                .map(|r| r.unwrap())
            {
                container.insert(device_id, keys);
            }
            device_keys.insert(user_id.clone(), container);
        } else {
            for device_id in device_ids {
                let mut container = BTreeMap::new();
                for keys in db.users.get_device_keys(&user_id.clone(), &device_id) {
                    container.insert(device_id.clone(), keys.unwrap());
                }
                device_keys.insert(user_id.clone(), container);
            }
        }
    }

    MatrixResult(Ok(get_keys::Response {
        failures: BTreeMap::new(),
        device_keys,
    }))
}

#[post("/_matrix/client/r0/keys/claim", data = "<body>")]
pub fn claim_keys_route(
    db: State<'_, Database>,
    body: Ruma<claim_keys::Request>,
) -> MatrixResult<claim_keys::Response> {
    let mut one_time_keys = BTreeMap::new();
    for (user_id, map) in &body.one_time_keys {
        let mut container = BTreeMap::new();
        for (device_id, key_algorithm) in map {
            if let Some(one_time_keys) = db
                .users
                .take_one_time_key(user_id, device_id, key_algorithm)
                .unwrap()
            {
                let mut c = BTreeMap::new();
                c.insert(one_time_keys.0, one_time_keys.1);
                container.insert(device_id.clone(), c);
            }
        }
        one_time_keys.insert(user_id.clone(), container);
    }

    MatrixResult(Ok(claim_keys::Response {
        failures: BTreeMap::new(),
        one_time_keys,
    }))
}

#[post("/_matrix/client/r0/rooms/<_room_id>/read_markers", data = "<body>")]
pub fn set_read_marker_route(
    db: State<'_, Database>,
    body: Ruma<set_read_marker::Request>,
    _room_id: String,
) -> MatrixResult<set_read_marker::Response> {
    let user_id = body.user_id.as_ref().expect("user is authenticated");

    db.account_data
        .update(
            Some(&body.room_id),
            &user_id,
            &EventType::FullyRead,
            serde_json::to_value(ruma_events::fully_read::FullyReadEvent {
                content: ruma_events::fully_read::FullyReadEventContent {
                    event_id: body.fully_read.clone(),
                },
                room_id: Some(body.room_id.clone()),
            })
            .unwrap()
            .as_object_mut()
            .unwrap(),
            &db.globals,
        )
        .unwrap();

    if let Some(event) = &body.read_receipt {
        db.rooms
            .edus
            .room_read_set(
                &body.room_id,
                &user_id,
                db.rooms
                    .get_pdu_count(event)
                    .unwrap()
                    .expect("TODO: what if a client specifies an invalid event"),
            )
            .unwrap();

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

        db.rooms
            .edus
            .roomlatest_update(
                &user_id,
                &body.room_id,
                EduEvent::Receipt(ruma_events::receipt::ReceiptEvent {
                    content: receipt_content,
                    room_id: None, // None because it can be inferred
                }),
                &db.globals,
            )
            .unwrap();
    }
    MatrixResult(Ok(set_read_marker::Response))
}

#[put(
    "/_matrix/client/r0/rooms/<_room_id>/typing/<_user_id>",
    data = "<body>"
)]
pub fn create_typing_event_route(
    db: State<'_, Database>,
    body: Ruma<create_typing_event::Request>,
    _room_id: String,
    _user_id: String,
) -> MatrixResult<create_typing_event::Response> {
    let user_id = body.user_id.as_ref().expect("user is authenticated");
    let edu = EduEvent::Typing(ruma_events::typing::TypingEvent {
        content: ruma_events::typing::TypingEventContent {
            user_ids: vec![user_id.clone()],
        },
        room_id: Some(body.room_id.clone()), // TODO: Can be None because it can be inferred
    });

    if body.typing {
        db.rooms
            .edus
            .roomactive_add(
                edu,
                &body.room_id,
                body.timeout.map(|d| d.as_millis() as u64).unwrap_or(30000)
                    + utils::millis_since_unix_epoch().try_into().unwrap_or(0),
                &db.globals,
            )
            .unwrap();
    } else {
        db.rooms.edus.roomactive_remove(edu, &body.room_id).unwrap();
    }

    MatrixResult(Ok(create_typing_event::Response))
}

#[post("/_matrix/client/r0/createRoom", data = "<body>")]
pub fn create_room_route(
    db: State<'_, Database>,
    body: Ruma<create_room::Request>,
) -> MatrixResult<create_room::Response> {
    // TODO: check if room is unique
    let room_id = RoomId::new(db.globals.server_name()).expect("host is valid");
    let user_id = body.user_id.as_ref().expect("user is authenticated");

    db.rooms
        .append_pdu(
            room_id.clone(),
            user_id.clone(),
            EventType::RoomCreate,
            json!({ "creator": user_id }),
            None,
            Some("".to_owned()),
            &db.globals,
        )
        .unwrap();

    db.rooms
        .join(
            &room_id,
            &user_id,
            db.users.displayname(&user_id).unwrap(),
            &db.globals,
        )
        .unwrap();

    db.rooms
        .append_pdu(
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
            &db.globals,
        )
        .unwrap();

    if let Some(name) = &body.name {
        db.rooms
            .append_pdu(
                room_id.clone(),
                user_id.clone(),
                EventType::RoomName,
                json!({ "name": name }),
                None,
                Some("".to_owned()),
                &db.globals,
            )
            .unwrap();
    }

    if let Some(topic) = &body.topic {
        db.rooms
            .append_pdu(
                room_id.clone(),
                user_id.clone(),
                EventType::RoomTopic,
                json!({ "topic": topic }),
                None,
                Some("".to_owned()),
                &db.globals,
            )
            .unwrap();
    }

    for user in &body.invite {
        db.rooms
            .invite(&user_id, &room_id, user, &db.globals)
            .unwrap();
    }

    MatrixResult(Ok(create_room::Response { room_id }))
}

#[get("/_matrix/client/r0/directory/room/<_room_alias>", data = "<body>")]
pub fn get_alias_route(
    db: State<'_, Database>,
    body: Ruma<get_alias::Request>,
    _room_alias: String,
) -> MatrixResult<get_alias::Response> {
    warn!("TODO: get_alias_route");
    let room_id = if body.room_alias.server_name() == db.globals.server_name() {
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
    db: State<'_, Database>,
    body: Ruma<join_room_by_id::Request>,
    _room_id: String,
) -> MatrixResult<join_room_by_id::Response> {
    let user_id = body.user_id.as_ref().expect("user is authenticated");

    if db
        .rooms
        .join(
            &body.room_id,
            &user_id,
            db.users.displayname(&user_id).unwrap(),
            &db.globals,
        )
        .is_ok()
    {
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
    db: State<'_, Database>,
    body: Ruma<join_room_by_id_or_alias::Request>,
    _room_id_or_alias: String,
) -> MatrixResult<join_room_by_id_or_alias::Response> {
    let user_id = body.user_id.as_ref().expect("user is authenticated");

    let room_id = match RoomId::try_from(body.room_id_or_alias.clone()) {
        Ok(room_id) => room_id,
        Err(room_alias) => {
            if room_alias.server_name() == db.globals.server_name() {
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
        }
    };

    if db
        .rooms
        .join(
            &room_id,
            &user_id,
            db.users.displayname(&user_id).unwrap(),
            &db.globals,
        )
        .is_ok()
    {
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
    db: State<'_, Database>,
    body: Ruma<leave_room::Request>,
    _room_id: String,
) -> MatrixResult<leave_room::Response> {
    let user_id = body.user_id.as_ref().expect("user is authenticated");
    db.rooms
        .leave(&user_id, &body.room_id, &user_id, &db.globals)
        .unwrap();
    MatrixResult(Ok(leave_room::Response))
}

#[post("/_matrix/client/r0/rooms/<_room_id>/forget", data = "<body>")]
pub fn forget_room_route(
    db: State<'_, Database>,
    body: Ruma<forget_room::Request>,
    _room_id: String,
) -> MatrixResult<forget_room::Response> {
    let user_id = body.user_id.as_ref().expect("user is authenticated");
    db.rooms.forget(&body.room_id, &user_id).unwrap();
    MatrixResult(Ok(forget_room::Response))
}

#[post("/_matrix/client/r0/rooms/<_room_id>/invite", data = "<body>")]
pub fn invite_user_route(
    db: State<'_, Database>,
    body: Ruma<invite_user::Request>,
    _room_id: String,
) -> MatrixResult<invite_user::Response> {
    if let invite_user::InvitationRecipient::UserId { user_id } = &body.recipient {
        db.rooms
            .invite(
                &body.user_id.as_ref().expect("user is authenticated"),
                &body.room_id,
                &user_id,
                &db.globals,
            )
            .unwrap();
        MatrixResult(Ok(invite_user::Response))
    } else {
        MatrixResult(Err(Error {
            kind: ErrorKind::NotFound,
            message: "User not found.".to_owned(),
            status_code: http::StatusCode::NOT_FOUND,
        }))
    }
}

#[post("/_matrix/client/r0/publicRooms")]
pub async fn get_public_rooms_filtered_route(
    db: State<'_, Database>,
) -> MatrixResult<get_public_rooms_filtered::Response> {
    let mut chunk = db
        .rooms
        .all_rooms()
        .into_iter()
        .map(|room_id| {
            let state = db.rooms.room_state(&room_id).unwrap();
            directory::PublicRoomsChunk {
                aliases: Vec::new(),
                canonical_alias: None,
                name: state
                    .get(&(EventType::RoomName, "".to_owned()))
                    .and_then(|s| s.content.get("name"))
                    .and_then(|n| n.as_str())
                    .map(|n| n.to_owned()),
                num_joined_members: (db.rooms.room_members(&room_id).count() as u32).into(),
                room_id,
                topic: None,
                world_readable: false,
                guest_can_join: true,
                avatar_url: None,
            }
        })
        .collect::<Vec<_>>();

    chunk.sort_by(|l, r| r.num_joined_members.cmp(&l.num_joined_members));

    /*
    chunk.extend_from_slice(
        &server_server::send_request(
            &db,
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
    */

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
    db: State<'_, Database>,
    body: Ruma<search_users::Request>,
) -> MatrixResult<search_users::Response> {
    MatrixResult(Ok(search_users::Response {
        results: db
            .users
            .iter()
            .map(Result::unwrap)
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

#[get("/_matrix/client/r0/rooms/<_room_id>/members")]
pub fn get_member_events_route(_room_id: String) -> MatrixResult<get_member_events::Response> {
    warn!("TODO: get_member_events_route");
    MatrixResult(Ok(get_member_events::Response { chunk: Vec::new() }))
}

#[get("/_matrix/client/r0/thirdparty/protocols")]
pub fn get_protocols_route() -> MatrixResult<get_protocols::Response> {
    warn!("TODO: get_protocols_route");
    MatrixResult(Ok(get_protocols::Response {
        protocols: BTreeMap::new(),
    }))
}

#[put(
    "/_matrix/client/r0/rooms/<_room_id>/send/<_event_type>/<_txn_id>",
    data = "<body>"
)]
pub fn create_message_event_route(
    db: State<'_, Database>,
    body: Ruma<create_message_event::Request>,
    _room_id: String,
    _event_type: String,
    _txn_id: String,
) -> MatrixResult<create_message_event::Response> {
    let user_id = body.user_id.as_ref().expect("user is authenticated");

    let mut unsigned = serde_json::Map::new();
    unsigned.insert("transaction_id".to_owned(), body.txn_id.clone().into());

    let event_id = db
        .rooms
        .append_pdu(
            body.room_id.clone(),
            user_id.clone(),
            body.event_type.clone(),
            body.json_body.clone(),
            Some(unsigned),
            None,
            &db.globals,
        )
        .expect("message events are always okay");

    MatrixResult(Ok(create_message_event::Response {
        event_id: Some(event_id),
    }))
}

#[put(
    "/_matrix/client/r0/rooms/<_room_id>/state/<_event_type>/<_state_key>",
    data = "<body>"
)]
pub fn create_state_event_for_key_route(
    db: State<'_, Database>,
    body: Ruma<create_state_event_for_key::Request>,
    _room_id: String,
    _event_type: String,
    _state_key: String,
) -> MatrixResult<create_state_event_for_key::Response> {
    let user_id = body.user_id.as_ref().expect("user is authenticated");

    // Reponse of with/without key is the same
    let event_id = db
        .rooms
        .append_pdu(
            body.room_id.clone(),
            user_id.clone(),
            body.event_type.clone(),
            body.json_body.clone(),
            None,
            Some(body.state_key.clone()),
            &db.globals,
        )
        .unwrap();

    MatrixResult(Ok(create_state_event_for_key::Response {
        event_id: Some(event_id),
    }))
}

#[put(
    "/_matrix/client/r0/rooms/<_room_id>/state/<_event_type>",
    data = "<body>"
)]
pub fn create_state_event_for_empty_key_route(
    db: State<'_, Database>,
    body: Ruma<create_state_event_for_empty_key::Request>,
    _room_id: String,
    _event_type: String,
) -> MatrixResult<create_state_event_for_empty_key::Response> {
    let user_id = body.user_id.as_ref().expect("user is authenticated");

    // Reponse of with/without key is the same
    let event_id = db
        .rooms
        .append_pdu(
            body.room_id.clone(),
            user_id.clone(),
            body.event_type.clone(),
            body.json_body.clone(),
            None,
            Some("".to_owned()),
            &db.globals,
        )
        .unwrap();

    MatrixResult(Ok(create_state_event_for_empty_key::Response {
        event_id: Some(event_id),
    }))
}

#[get("/_matrix/client/r0/rooms/<_room_id>/state", data = "<body>")]
pub fn get_state_events_route(
    db: State<'_, Database>,
    body: Ruma<get_state_events::Request>,
    _room_id: String,
) -> MatrixResult<get_state_events::Response> {
    MatrixResult(Ok(get_state_events::Response {
        room_state: db
            .rooms
            .room_state(&body.room_id)
            .unwrap()
            .values()
            .map(|pdu| pdu.to_state_event())
            .collect(),
    }))
}

#[get(
    "/_matrix/client/r0/rooms/<_room_id>/state/<_event_type>/<_state_key>",
    data = "<body>"
)]
pub fn get_state_events_for_key_route(
    db: State<'_, Database>,
    body: Ruma<get_state_events_for_key::Request>,
    _room_id: String,
    _event_type: String,
    _state_key: String,
) -> MatrixResult<get_state_events_for_key::Response> {
    if let Some(event) = db
        .rooms
        .room_state(&body.room_id)
        .unwrap()
        .get(&(body.event_type.clone(), body.state_key.clone()))
    {
        MatrixResult(Ok(get_state_events_for_key::Response {
            content: serde_json::value::to_raw_value(event).unwrap(),
        }))
    } else {
        MatrixResult(Err(Error {
            kind: ErrorKind::NotFound,
            message: "State event not found.".to_owned(),
            status_code: http::StatusCode::BAD_REQUEST,
        }))
    }
}

#[get(
    "/_matrix/client/r0/rooms/<_room_id>/state/<_event_type>",
    data = "<body>"
)]
pub fn get_state_events_for_empty_key_route(
    db: State<'_, Database>,
    body: Ruma<get_state_events_for_empty_key::Request>,
    _room_id: String,
    _event_type: String,
) -> MatrixResult<get_state_events_for_key::Response> {
    if let Some(event) = db
        .rooms
        .room_state(&body.room_id)
        .unwrap()
        .get(&(body.event_type.clone(), "".to_owned()))
    {
        MatrixResult(Ok(get_state_events_for_key::Response {
            content: serde_json::value::to_raw_value(event).unwrap(),
        }))
    } else {
        MatrixResult(Err(Error {
            kind: ErrorKind::NotFound,
            message: "State event not found.".to_owned(),
            status_code: http::StatusCode::BAD_REQUEST,
        }))
    }
}

#[get("/_matrix/client/r0/sync", data = "<body>")]
pub fn sync_route(
    db: State<'_, Database>,
    body: Ruma<sync_events::Request>,
) -> MatrixResult<sync_events::Response> {
    std::thread::sleep(Duration::from_millis(1000));
    let user_id = body.user_id.as_ref().expect("user is authenticated");
    let device_id = body.device_id.as_ref().expect("user is authenticated");

    let next_batch = db.globals.current_count().unwrap().to_string();

    let mut joined_rooms = BTreeMap::new();
    let since = body
        .since
        .clone()
        .and_then(|string| string.parse().ok())
        .unwrap_or(0);

    for room_id in db.rooms.rooms_joined(&user_id) {
        let room_id = room_id.unwrap();

        let mut pdus = db
            .rooms
            .pdus_since(&room_id, since)
            .unwrap()
            .map(|r| r.unwrap())
            .collect::<Vec<_>>();

        let mut send_member_count = false;
        let mut send_full_state = false;
        for pdu in &pdus {
            if pdu.kind == EventType::RoomMember {
                if pdu.state_key == Some(user_id.to_string()) && pdu.content["membership"] == "join"
                {
                    send_full_state = true;
                }
                send_member_count = true;
            }
        }

        let notification_count =
            if let Some(last_read) = db.rooms.edus.room_read_get(&room_id, &user_id).unwrap() {
                Some((db.rooms.pdus_since(&room_id, last_read).unwrap().count() as u32).into())
            } else {
                None
            };

        // They /sync response doesn't always return all messages, so we say the output is
        // limited unless there are enough events
        let mut limited = true;
        pdus = pdus.split_off(pdus.len().checked_sub(10).unwrap_or_else(|| {
            limited = false;
            0
        }));

        let prev_batch = pdus
            .first()
            .and_then(|e| db.rooms.get_pdu_count(&e.event_id).unwrap())
            .map(|c| c.to_string());

        let room_events = pdus
            .into_iter()
            .map(|pdu| pdu.to_room_event())
            .collect::<Vec<_>>();

        let mut edus = db
            .rooms
            .edus
            .roomactives_all(&room_id)
            .map(|r| r.unwrap())
            .collect::<Vec<_>>();

        if edus.is_empty() {
            edus.push(
                EduEvent::Typing(ruma_events::typing::TypingEvent {
                    content: ruma_events::typing::TypingEventContent {
                        user_ids: Vec::new(),
                    },
                    room_id: Some(room_id.clone()), // None because it can be inferred
                })
                .into(),
            );
        }

        edus.extend(
            db.rooms
                .edus
                .roomlatests_since(&room_id, since)
                .unwrap()
                .map(|r| r.unwrap()),
        );

        joined_rooms.insert(
            room_id.clone().try_into().unwrap(),
            sync_events::JoinedRoom {
                account_data: Some(sync_events::AccountData {
                    events: db
                        .account_data
                        .changes_since(Some(&room_id), &user_id, since)
                        .unwrap()
                        .into_iter()
                        .map(|(_, v)| v)
                        .collect(),
                }),
                summary: sync_events::RoomSummary {
                    heroes: Vec::new(),
                    joined_member_count: if send_member_count {
                        Some((db.rooms.room_members(&room_id).count() as u32).into())
                    } else {
                        None
                    },
                    invited_member_count: if send_member_count {
                        Some((db.rooms.room_members_invited(&room_id).count() as u32).into())
                    } else {
                        None
                    },
                },
                unread_notifications: sync_events::UnreadNotificationsCount {
                    highlight_count: None,
                    notification_count,
                },
                timeline: sync_events::Timeline {
                    limited: if limited { Some(limited) } else { None },
                    prev_batch,
                    events: room_events,
                },
                // TODO: state before timeline
                state: sync_events::State {
                    events: if send_full_state {
                        db.rooms
                            .room_state(&room_id)
                            .unwrap()
                            .into_iter()
                            .map(|(_, pdu)| pdu.to_state_event())
                            .collect()
                    } else {
                        Vec::new()
                    },
                },
                ephemeral: sync_events::Ephemeral { events: edus },
            },
        );
    }

    let mut left_rooms = BTreeMap::new();
    for room_id in db.rooms.rooms_left(&user_id) {
        let room_id = room_id.unwrap();
        let pdus = db.rooms.pdus_since(&room_id, since).unwrap();
        let room_events = pdus.map(|pdu| pdu.unwrap().to_room_event()).collect();

        let mut edus = db
            .rooms
            .edus
            .roomlatests_since(&room_id, since)
            .unwrap()
            .map(|r| r.unwrap())
            .collect::<Vec<_>>();

        edus.extend(db.rooms.edus.roomactives_all(&room_id).map(|r| r.unwrap()));

        left_rooms.insert(
            room_id.clone().try_into().unwrap(),
            sync_events::LeftRoom {
                account_data: Some(sync_events::AccountData { events: Vec::new() }),
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
    for room_id in db.rooms.rooms_invited(&user_id) {
        let room_id = room_id.unwrap();
        let events = db
            .rooms
            .pdus_since(&room_id, since)
            .unwrap()
            .map(|pdu| pdu.unwrap().to_stripped_state_event())
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
        presence: sync_events::Presence {
            events: db
                .global_edus
                .globallatests_since(since)
                .unwrap()
                .filter_map(|edu| {
                    // Only look for presence events
                    if let Ok(mut edu) = EventJson::<ruma_events::presence::PresenceEvent>::from(
                        edu.unwrap().into_json(),
                    )
                    .deserialize()
                    {
                        let timestamp = edu.content.last_active_ago.unwrap();
                        edu.content.last_active_ago = Some(
                            js_int::UInt::try_from(utils::millis_since_unix_epoch()).unwrap()
                                - timestamp,
                        );
                        Some(edu.into())
                    } else {
                        None
                    }
                })
                .collect(),
        },
        account_data: sync_events::AccountData {
            events: db
                .account_data
                .changes_since(None, &user_id, since)
                .unwrap()
                .into_iter()
                .map(|(_, v)| v)
                .collect(),
        },
        device_lists: Default::default(),
        device_one_time_keys_count: Default::default(),
        to_device: sync_events::ToDevice {
            events: db
                .users
                .take_to_device_events(user_id, device_id, 100)
                .unwrap(),
        },
    }))
}

#[get("/_matrix/client/r0/rooms/<_room_id>/messages", data = "<body>")]
pub fn get_message_events_route(
    db: State<'_, Database>,
    body: Ruma<get_message_events::Request>,
    _room_id: String,
) -> MatrixResult<get_message_events::Response> {
    if let get_message_events::Direction::Forward = body.dir {
        todo!();
    }

    if let Ok(from) = body.from.clone().parse() {
        let pdus = db
            .rooms
            .pdus_until(&body.room_id, from)
            .take(body.limit.map(|l| l.try_into().unwrap()).unwrap_or(10_u32) as usize)
            .map(|r| r.unwrap())
            .collect::<Vec<_>>();
        let prev_batch = pdus
            .last()
            .and_then(|e| db.rooms.get_pdu_count(&e.event_id).unwrap())
            .map(|c| c.to_string());
        let room_events = pdus
            .into_iter()
            .map(|pdu| pdu.to_room_event())
            .collect::<Vec<_>>();

        MatrixResult(Ok(get_message_events::Response {
            start: Some(body.from.clone()),
            end: prev_batch,
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
    warn!("TODO: turn_server_route");
    MatrixResult(Err(Error {
        kind: ErrorKind::NotFound,
        message: "There is no turn server yet.".to_owned(),
        status_code: http::StatusCode::NOT_FOUND,
    }))
}

#[post("/_matrix/client/r0/publicised_groups")]
pub fn publicised_groups_route() -> MatrixResult<create_message_event::Response> {
    warn!("TODO: publicised_groups_route");
    MatrixResult(Err(Error {
        kind: ErrorKind::NotFound,
        message: "There are no publicised groups yet.".to_owned(),
        status_code: http::StatusCode::NOT_FOUND,
    }))
}

#[put(
    "/_matrix/client/r0/sendToDevice/<_event_type>/<_txn_id>",
    data = "<body>"
)]
pub fn send_event_to_device_route(
    db: State<'_, Database>,
    body: Ruma<send_event_to_device::Request>,
    _event_type: String,
    _txn_id: String,
) -> MatrixResult<send_event_to_device::Response> {
    let user_id = body.user_id.as_ref().expect("user is authenticated");

    for (target_user_id, map) in &body.messages {
        for (target_device_id_maybe, event) in map {
            match target_device_id_maybe {
                to_device::DeviceIdOrAllDevices::DeviceId(target_device_id) => db
                    .users
                    .add_to_device_event(
                        user_id,
                        &target_user_id,
                        &target_device_id,
                        &body.event_type,
                        serde_json::from_str(event.get()).unwrap(),
                        &db.globals,
                    )
                    .unwrap(),

                to_device::DeviceIdOrAllDevices::AllDevices => {
                    for target_device_id in db.users.all_device_ids(&target_user_id) {
                        target_device_id.unwrap();
                    }
                }
            }
        }
    }

    MatrixResult(Ok(send_event_to_device::Response))
}

#[get("/_matrix/media/r0/config")]
pub fn get_media_config_route() -> MatrixResult<get_media_config::Response> {
    warn!("TODO: get_media_config_route");
    MatrixResult(Ok(get_media_config::Response {
        upload_size: 0_u32.into(),
    }))
}

#[options("/<_segments..>")]
pub fn options_route(
    _segments: rocket::http::uri::Segments<'_>,
) -> MatrixResult<create_message_event::Response> {
    MatrixResult(Err(Error {
        kind: ErrorKind::NotFound,
        message: "".to_owned(),
        status_code: http::StatusCode::OK,
    }))
}
