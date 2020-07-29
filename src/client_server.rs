use std::{
    collections::{hash_map, BTreeMap, HashMap, HashSet},
    convert::{TryFrom, TryInto},
    time::{Duration, SystemTime},
};

use crate::{utils, ConduitResult, Database, Error, Ruma};
use keys::{upload_signatures, upload_signing_keys};
use log::warn;

#[cfg(not(feature = "conduit_bin"))]
use super::State;
#[cfg(feature = "conduit_bin")]
use rocket::{delete, get, options, post, put, tokio, State};

use ruma::{
    api::client::{
        error::ErrorKind,
        r0::{
            account::{
                change_password, deactivate, get_username_availability, register, whoami,
                ThirdPartyIdRemovalStatus,
            },
            alias::{create_alias, delete_alias, get_alias},
            backup::{
                add_backup_keys, create_backup, get_backup, get_backup_keys, get_latest_backup,
                update_backup,
            },
            capabilities::get_capabilities,
            config::{get_global_account_data, set_global_account_data},
            context::get_context,
            device::{self, delete_device, delete_devices, get_device, get_devices, update_device},
            directory::{
                self, get_public_rooms, get_public_rooms_filtered, get_room_visibility,
                set_room_visibility,
            },
            filter::{self, create_filter, get_filter},
            keys::{self, claim_keys, get_key_changes, get_keys, upload_keys},
            media::{create_content, get_content, get_content_thumbnail, get_media_config},
            membership::{
                ban_user, forget_room, get_member_events, invite_user, join_room_by_id,
                join_room_by_id_or_alias, joined_members, joined_rooms, kick_user, leave_room,
                unban_user,
            },
            message::{create_message_event, get_message_events},
            presence::set_presence,
            profile::{
                get_avatar_url, get_display_name, get_profile, set_avatar_url, set_display_name,
            },
            push::{get_pushers, get_pushrules_all, set_pushrule, set_pushrule_enabled},
            read_marker::set_read_marker,
            redact::redact_event,
            room::{self, create_room, get_room_event},
            session::{get_login_types, login, logout, logout_all},
            state::{
                create_state_event_for_empty_key, create_state_event_for_key, get_state_events,
                get_state_events_for_empty_key, get_state_events_for_key,
            },
            sync::sync_events,
            tag::{create_tag, delete_tag, get_tags},
            thirdparty::get_protocols,
            to_device::{self, send_event_to_device},
            typing::create_typing_event,
            uiaa::{AuthFlow, UiaaInfo},
            user_directory::search_users,
        },
        unversioned::get_supported_versions,
    },
    events::{
        custom::CustomEventContent,
        room::{
            canonical_alias, guest_access, history_visibility, join_rules, member, name, redaction,
            topic,
        },
        AnyEphemeralRoomEvent, AnyEvent, AnySyncEphemeralRoomEvent, BasicEvent, EventType,
    },
    Raw, RoomAliasId, RoomId, RoomVersionId, UserId,
};

const GUEST_NAME_LENGTH: usize = 10;
const DEVICE_ID_LENGTH: usize = 10;
const TOKEN_LENGTH: usize = 256;
const MXC_LENGTH: usize = 256;
const SESSION_ID_LENGTH: usize = 256;

#[cfg_attr(feature = "conduit_bin", get("/_matrix/client/versions"))]
pub fn get_supported_versions_route() -> ConduitResult<get_supported_versions::Response> {
    let mut unstable_features = BTreeMap::new();

    unstable_features.insert("org.matrix.e2e_cross_signing".to_owned(), true);

    Ok(get_supported_versions::Response {
        versions: vec!["r0.5.0".to_owned(), "r0.6.0".to_owned()],
        unstable_features,
    }
    .into())
}

#[cfg_attr(
    feature = "conduit_bin",
    get("/_matrix/client/r0/register/available", data = "<body>")
)]
pub fn get_register_available_route(
    db: State<'_, Database>,
    body: Ruma<get_username_availability::Request>,
) -> ConduitResult<get_username_availability::Response> {
    // Validate user id
    let user_id = UserId::parse_with_server_name(body.username.clone(), db.globals.server_name())
        .ok()
        .filter(|user_id| {
            !user_id.is_historical() && user_id.server_name() == db.globals.server_name()
        })
        .ok_or(Error::BadRequest(
            ErrorKind::InvalidUsername,
            "Username is invalid.",
        ))?;

    // Check if username is creative enough
    if db.users.exists(&user_id)? {
        return Err(Error::BadRequest(
            ErrorKind::UserInUse,
            "Desired user ID is already taken.",
        ));
    }

    // TODO add check for appservice namespaces

    // If no if check is true we have an username that's available to be used.
    Ok(get_username_availability::Response { available: true }.into())
}

#[cfg_attr(
    feature = "conduit_bin",
    post("/_matrix/client/r0/register", data = "<body>")
)]
pub fn register_route(
    db: State<'_, Database>,
    body: Ruma<register::Request>,
) -> ConduitResult<register::Response> {
    if db.globals.registration_disabled() {
        return Err(Error::BadRequest(
            ErrorKind::Forbidden,
            "Registration has been disabled.",
        ));
    }

    // Validate user id
    let user_id = UserId::parse_with_server_name(
        body.username
            .clone()
            .unwrap_or_else(|| utils::random_string(GUEST_NAME_LENGTH))
            .to_lowercase(),
        db.globals.server_name(),
    )
    .ok()
    .filter(|user_id| !user_id.is_historical() && user_id.server_name() == db.globals.server_name())
    .ok_or(Error::BadRequest(
        ErrorKind::InvalidUsername,
        "Username is invalid.",
    ))?;

    // Check if username is creative enough
    if db.users.exists(&user_id)? {
        return Err(Error::BadRequest(
            ErrorKind::UserInUse,
            "Desired user ID is already taken.",
        ));
    }

    // UIAA
    let mut uiaainfo = UiaaInfo {
        flows: vec![AuthFlow {
            stages: vec!["m.login.dummy".to_owned()],
        }],
        completed: Vec::new(),
        params: Default::default(),
        session: None,
        auth_error: None,
    };

    if let Some(auth) = &body.auth {
        let (worked, uiaainfo) =
            db.uiaa
                .try_auth(&user_id, "".into(), auth, &uiaainfo, &db.users, &db.globals)?;
        if !worked {
            return Err(Error::Uiaa(uiaainfo));
        }
    // Success!
    } else {
        uiaainfo.session = Some(utils::random_string(SESSION_ID_LENGTH));
        db.uiaa.create(&user_id, "".into(), &uiaainfo)?;
        return Err(Error::Uiaa(uiaainfo));
    }

    let password = body.password.clone().unwrap_or_default();

    // Create user
    db.users.create(&user_id, &password)?;

    // Generate new device id if the user didn't specify one
    let device_id = body
        .device_id
        .clone()
        .unwrap_or_else(|| utils::random_string(DEVICE_ID_LENGTH).into());

    // Generate new token for the device
    let token = utils::random_string(TOKEN_LENGTH);

    // Add device
    db.users.create_device(
        &user_id,
        &device_id,
        &token,
        body.initial_device_display_name.clone(),
    )?;

    // Initial data
    db.account_data.update(
        None,
        &user_id,
        EventType::PushRules,
        &ruma::events::push_rules::PushRulesEvent {
            content: ruma::events::push_rules::PushRulesEventContent {
                global: crate::push_rules::default_pushrules(&user_id),
            },
        },
        &db.globals,
    )?;

    Ok(register::Response {
        access_token: Some(token),
        user_id,
        device_id: Some(device_id.into()),
    }
    .into())
}

#[cfg_attr(feature = "conduit_bin", get("/_matrix/client/r0/login"))]
pub fn get_login_route() -> ConduitResult<get_login_types::Response> {
    Ok(get_login_types::Response {
        flows: vec![get_login_types::LoginType::Password],
    }
    .into())
}

#[cfg_attr(
    feature = "conduit_bin",
    post("/_matrix/client/r0/login", data = "<body>")
)]
pub fn login_route(
    db: State<'_, Database>,
    body: Ruma<login::Request>,
) -> ConduitResult<login::Response> {
    // Validate login method
    let user_id =
        // TODO: Other login methods
        if let (login::UserInfo::MatrixId(username), login::LoginInfo::Password { password }) =
            (body.user.clone(), body.login_info.clone())
        {
            let user_id = UserId::parse_with_server_name(username, db.globals.server_name()).map_err(|_| Error::BadRequest(ErrorKind::InvalidUsername, "Username is invalid."))?;
            let hash = db.users.password_hash(&user_id)?.ok_or(Error::BadRequest(ErrorKind::Forbidden, "Wrong username or password."))?;

            if hash.is_empty() {
                return Err(Error::BadRequest(ErrorKind::UserDeactivated, "The user has been deactivated"));
            }

            let hash_matches =
                argon2::verify_encoded(&hash, password.as_bytes()).unwrap_or(false);

            if !hash_matches {
                return Err(Error::BadRequest(ErrorKind::Forbidden, "Wrong username or password."));
            }

            user_id
        } else {
            return Err(Error::BadRequest(ErrorKind::Forbidden, "Bad login type."));
        };

    // Generate new device id if the user didn't specify one
    let device_id = body
        .body
        .device_id
        .clone()
        .unwrap_or_else(|| utils::random_string(DEVICE_ID_LENGTH).into());

    // Generate a new token for the device
    let token = utils::random_string(TOKEN_LENGTH);

    // Add device
    db.users.create_device(
        &user_id,
        &device_id,
        &token,
        body.initial_device_display_name.clone(),
    )?;

    Ok(login::Response {
        user_id,
        access_token: token,
        home_server: Some(db.globals.server_name().to_owned()),
        device_id: device_id.into(),
        well_known: None,
    }
    .into())
}

#[cfg_attr(
    feature = "conduit_bin",
    get("/_matrix/client/r0/account/whoami", data = "<body>")
)]
pub fn whoami_route(body: Ruma<whoami::Request>) -> ConduitResult<whoami::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");
    Ok(whoami::Response {
        user_id: sender_id.clone(),
    }
    .into())
}

#[cfg_attr(
    feature = "conduit_bin",
    post("/_matrix/client/r0/logout", data = "<body>")
)]
pub fn logout_route(
    db: State<'_, Database>,
    body: Ruma<logout::Request>,
) -> ConduitResult<logout::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");
    let device_id = body.device_id.as_ref().expect("user is authenticated");

    db.users.remove_device(&sender_id, device_id)?;

    Ok(logout::Response.into())
}

#[cfg_attr(
    feature = "conduit_bin",
    post("/_matrix/client/r0/logout/all", data = "<body>")
)]
pub fn logout_all_route(
    db: State<'_, Database>,
    body: Ruma<logout_all::Request>,
) -> ConduitResult<logout_all::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");

    for device_id in db.users.all_device_ids(sender_id) {
        if let Ok(device_id) = device_id {
            db.users.remove_device(&sender_id, &device_id)?;
        }
    }

    Ok(logout_all::Response.into())
}

#[cfg_attr(
    feature = "conduit_bin",
    post("/_matrix/client/r0/account/password", data = "<body>")
)]
pub fn change_password_route(
    db: State<'_, Database>,
    body: Ruma<change_password::Request>,
) -> ConduitResult<change_password::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");
    let device_id = body.device_id.as_ref().expect("user is authenticated");

    let mut uiaainfo = UiaaInfo {
        flows: vec![AuthFlow {
            stages: vec!["m.login.password".to_owned()],
        }],
        completed: Vec::new(),
        params: Default::default(),
        session: None,
        auth_error: None,
    };

    if let Some(auth) = &body.auth {
        let (worked, uiaainfo) = db.uiaa.try_auth(
            &sender_id,
            device_id,
            auth,
            &uiaainfo,
            &db.users,
            &db.globals,
        )?;
        if !worked {
            return Err(Error::Uiaa(uiaainfo));
        }
    // Success!
    } else {
        uiaainfo.session = Some(utils::random_string(SESSION_ID_LENGTH));
        db.uiaa.create(&sender_id, &device_id, &uiaainfo)?;
        return Err(Error::Uiaa(uiaainfo));
    }

    db.users.set_password(&sender_id, &body.new_password)?;

    // TODO: Read logout_devices field when it's available and respect that, currently not supported in Ruma
    // See: https://github.com/ruma/ruma/issues/107
    // Logout all devices except the current one
    for id in db
        .users
        .all_device_ids(&sender_id)
        .filter_map(|id| id.ok())
        .filter(|id| id != device_id)
    {
        db.users.remove_device(&sender_id, &id)?;
    }

    Ok(change_password::Response.into())
}

#[cfg_attr(
    feature = "conduit_bin",
    post("/_matrix/client/r0/account/deactivate", data = "<body>")
)]
pub fn deactivate_route(
    db: State<'_, Database>,
    body: Ruma<deactivate::Request>,
) -> ConduitResult<deactivate::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");
    let device_id = body.device_id.as_ref().expect("user is authenticated");

    let mut uiaainfo = UiaaInfo {
        flows: vec![AuthFlow {
            stages: vec!["m.login.password".to_owned()],
        }],
        completed: Vec::new(),
        params: Default::default(),
        session: None,
        auth_error: None,
    };

    if let Some(auth) = &body.auth {
        let (worked, uiaainfo) = db.uiaa.try_auth(
            &sender_id,
            &device_id,
            auth,
            &uiaainfo,
            &db.users,
            &db.globals,
        )?;
        if !worked {
            return Err(Error::Uiaa(uiaainfo));
        }
    // Success!
    } else {
        uiaainfo.session = Some(utils::random_string(SESSION_ID_LENGTH));
        db.uiaa.create(&sender_id, &device_id, &uiaainfo)?;
        return Err(Error::Uiaa(uiaainfo));
    }

    // Leave all joined rooms and reject all invitations
    for room_id in db
        .rooms
        .rooms_joined(&sender_id)
        .chain(db.rooms.rooms_invited(&sender_id))
    {
        let room_id = room_id?;
        let event = member::MemberEventContent {
            membership: member::MembershipState::Leave,
            displayname: None,
            avatar_url: None,
            is_direct: None,
            third_party_invite: None,
        };

        db.rooms.append_pdu(
            room_id.clone(),
            sender_id.clone(),
            EventType::RoomMember,
            serde_json::to_value(event).expect("event is valid, we just created it"),
            None,
            Some(sender_id.to_string()),
            None,
            &db.globals,
        )?;
    }

    // Remove devices and mark account as deactivated
    db.users.deactivate_account(&sender_id)?;

    Ok(deactivate::Response {
        id_server_unbind_result: ThirdPartyIdRemovalStatus::NoSupport,
    }
    .into())
}

#[cfg_attr(feature = "conduit_bin", get("/_matrix/client/r0/capabilities"))]
pub fn get_capabilities_route() -> ConduitResult<get_capabilities::Response> {
    let mut available = BTreeMap::new();
    available.insert(
        RoomVersionId::Version5,
        get_capabilities::RoomVersionStability::Stable,
    );
    available.insert(
        RoomVersionId::Version6,
        get_capabilities::RoomVersionStability::Stable,
    );

    Ok(get_capabilities::Response {
        capabilities: get_capabilities::Capabilities {
            change_password: None, // None means it is possible
            room_versions: Some(get_capabilities::RoomVersionsCapability {
                default: "6".to_owned(),
                available,
            }),
            custom_capabilities: BTreeMap::new(),
        },
    }
    .into())
}

#[cfg_attr(
    feature = "conduit_bin",
    get("/_matrix/client/r0/pushrules", data = "<body>")
)]
pub fn get_pushrules_all_route(
    db: State<'_, Database>,
    body: Ruma<get_pushrules_all::Request>,
) -> ConduitResult<get_pushrules_all::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");

    let event = db
        .account_data
        .get::<ruma::events::push_rules::PushRulesEvent>(None, &sender_id, EventType::PushRules)?
        .ok_or(Error::BadRequest(
            ErrorKind::NotFound,
            "PushRules event not found.",
        ))?;

    Ok(get_pushrules_all::Response {
        global: event.content.global,
    }
    .into())
}

#[cfg_attr(feature = "conduit_bin", put(
    "/_matrix/client/r0/pushrules/<_>/<_>/<_>",
    //data = "<body>"
))]
pub fn set_pushrule_route(//db: State<'_, Database>,
    //body: Ruma<set_pushrule::Request>,
) -> ConduitResult<set_pushrule::Response> {
    // TODO
    warn!("TODO: set_pushrule_route");
    Ok(set_pushrule::Response.into())
}

#[cfg_attr(
    feature = "conduit_bin",
    put("/_matrix/client/r0/pushrules/<_>/<_>/<_>/enabled")
)]
pub fn set_pushrule_enabled_route() -> ConduitResult<set_pushrule_enabled::Response> {
    // TODO
    warn!("TODO: set_pushrule_enabled_route");
    Ok(set_pushrule_enabled::Response.into())
}

#[cfg_attr(feature = "conduit_bin", get("/_matrix/client/r0/user/<_>/filter/<_>"))]
pub fn get_filter_route() -> ConduitResult<get_filter::Response> {
    // TODO
    Ok(get_filter::Response {
        filter: filter::FilterDefinition {
            event_fields: None,
            event_format: None,
            account_data: None,
            room: None,
            presence: None,
        },
    }
    .into())
}

#[cfg_attr(feature = "conduit_bin", post("/_matrix/client/r0/user/<_>/filter"))]
pub fn create_filter_route() -> ConduitResult<create_filter::Response> {
    // TODO
    Ok(create_filter::Response {
        filter_id: utils::random_string(10),
    }
    .into())
}

#[cfg_attr(
    feature = "conduit_bin",
    put("/_matrix/client/r0/user/<_>/account_data/<_>", data = "<body>")
)]
pub fn set_global_account_data_route(
    db: State<'_, Database>,
    body: Ruma<set_global_account_data::Request>,
) -> ConduitResult<set_global_account_data::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");

    let content = serde_json::from_str::<serde_json::Value>(body.data.get())
        .map_err(|_| Error::BadRequest(ErrorKind::BadJson, "Data is invalid."))?;

    let event_type = body.event_type.to_string();

    db.account_data.update(
        None,
        sender_id,
        event_type.clone().into(),
        &BasicEvent {
            content: CustomEventContent {
                event_type,
                json: content,
            },
        },
        &db.globals,
    )?;

    Ok(set_global_account_data::Response.into())
}

#[cfg_attr(
    feature = "conduit_bin",
    get("/_matrix/client/r0/user/<_>/account_data/<_>", data = "<body>")
)]
pub fn get_global_account_data_route(
    db: State<'_, Database>,
    body: Ruma<get_global_account_data::Request>,
) -> ConduitResult<get_global_account_data::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");

    let data = db
        .account_data
        .get::<Raw<ruma::events::AnyBasicEvent>>(
            None,
            sender_id,
            EventType::try_from(&body.event_type).expect("EventType::try_from can never fail"),
        )?
        .ok_or(Error::BadRequest(ErrorKind::NotFound, "Data not found."))?;

    Ok(get_global_account_data::Response { account_data: data }.into())
}

#[cfg_attr(
    feature = "conduit_bin",
    put("/_matrix/client/r0/profile/<_>/displayname", data = "<body>")
)]
pub fn set_displayname_route(
    db: State<'_, Database>,
    body: Ruma<set_display_name::Request>,
) -> ConduitResult<set_display_name::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");

    db.users
        .set_displayname(&sender_id, body.displayname.clone())?;

    // Send a new membership event and presence update into all joined rooms
    for room_id in db.rooms.rooms_joined(&sender_id) {
        let room_id = room_id?;
        db.rooms.append_pdu(
            room_id.clone(),
            sender_id.clone(),
            EventType::RoomMember,
            serde_json::to_value(ruma::events::room::member::MemberEventContent {
                displayname: body.displayname.clone(),
                ..serde_json::from_value::<Raw<_>>(
                    db.rooms
                        .room_state_get(&room_id, &EventType::RoomMember, &sender_id.to_string())?
                        .ok_or_else(|| {
                            Error::bad_database(
                                "Tried to send displayname update for user not in the room.",
                            )
                        })?
                        .content
                        .clone(),
                )
                .map_err(|_| Error::bad_database("Database contains invalid PDU."))?
                .deserialize()
                .map_err(|_| Error::bad_database("Database contains invalid PDU."))?
            })
            .expect("event is valid, we just created it"),
            None,
            Some(sender_id.to_string()),
            None,
            &db.globals,
        )?;

        // Presence update
        db.rooms.edus.update_presence(
            &sender_id,
            &room_id,
            ruma::events::presence::PresenceEvent {
                content: ruma::events::presence::PresenceEventContent {
                    avatar_url: db.users.avatar_url(&sender_id)?,
                    currently_active: None,
                    displayname: db.users.displayname(&sender_id)?,
                    last_active_ago: Some(
                        utils::millis_since_unix_epoch()
                            .try_into()
                            .expect("time is valid"),
                    ),
                    presence: ruma::presence::PresenceState::Online,
                    status_msg: None,
                },
                sender: sender_id.clone(),
            },
            &db.globals,
        )?;
    }

    Ok(set_display_name::Response.into())
}

#[cfg_attr(
    feature = "conduit_bin",
    get("/_matrix/client/r0/profile/<_>/displayname", data = "<body>")
)]
pub fn get_displayname_route(
    db: State<'_, Database>,
    body: Ruma<get_display_name::Request>,
) -> ConduitResult<get_display_name::Response> {
    Ok(get_display_name::Response {
        displayname: db.users.displayname(&body.user_id)?,
    }
    .into())
}

#[cfg_attr(
    feature = "conduit_bin",
    put("/_matrix/client/r0/profile/<_>/avatar_url", data = "<body>")
)]
pub fn set_avatar_url_route(
    db: State<'_, Database>,
    body: Ruma<set_avatar_url::Request>,
) -> ConduitResult<set_avatar_url::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");

    if let Some(avatar_url) = &body.avatar_url {
        if !avatar_url.starts_with("mxc://") {
            return Err(Error::BadRequest(
                ErrorKind::InvalidParam,
                "avatar_url has to start with mxc://.",
            ));
        }

        // TODO in the future when we can handle media uploads make sure that this url is our own server
        // TODO also make sure this is valid mxc:// format (not only starting with it)
    }

    db.users
        .set_avatar_url(&sender_id, body.avatar_url.clone())?;

    // Send a new membership event and presence update into all joined rooms
    for room_id in db.rooms.rooms_joined(&sender_id) {
        let room_id = room_id?;
        db.rooms.append_pdu(
            room_id.clone(),
            sender_id.clone(),
            EventType::RoomMember,
            serde_json::to_value(ruma::events::room::member::MemberEventContent {
                avatar_url: body.avatar_url.clone(),
                ..serde_json::from_value::<Raw<_>>(
                    db.rooms
                        .room_state_get(&room_id, &EventType::RoomMember, &sender_id.to_string())?
                        .ok_or_else(|| {
                            Error::bad_database(
                                "Tried to send avatar url update for user not in the room.",
                            )
                        })?
                        .content
                        .clone(),
                )
                .map_err(|_| Error::bad_database("Database contains invalid PDU."))?
                .deserialize()
                .map_err(|_| Error::bad_database("Database contains invalid PDU."))?
            })
            .expect("event is valid, we just created it"),
            None,
            Some(sender_id.to_string()),
            None,
            &db.globals,
        )?;

        // Presence update
        db.rooms.edus.update_presence(
            &sender_id,
            &room_id,
            ruma::events::presence::PresenceEvent {
                content: ruma::events::presence::PresenceEventContent {
                    avatar_url: db.users.avatar_url(&sender_id)?,
                    currently_active: None,
                    displayname: db.users.displayname(&sender_id)?,
                    last_active_ago: Some(
                        utils::millis_since_unix_epoch()
                            .try_into()
                            .expect("time is valid"),
                    ),
                    presence: ruma::presence::PresenceState::Online,
                    status_msg: None,
                },
                sender: sender_id.clone(),
            },
            &db.globals,
        )?;
    }

    Ok(set_avatar_url::Response.into())
}

#[cfg_attr(
    feature = "conduit_bin",
    get("/_matrix/client/r0/profile/<_>/avatar_url", data = "<body>")
)]
pub fn get_avatar_url_route(
    db: State<'_, Database>,
    body: Ruma<get_avatar_url::Request>,
) -> ConduitResult<get_avatar_url::Response> {
    Ok(get_avatar_url::Response {
        avatar_url: db.users.avatar_url(&body.user_id)?,
    }
    .into())
}

#[cfg_attr(
    feature = "conduit_bin",
    get("/_matrix/client/r0/profile/<_>", data = "<body>")
)]
pub fn get_profile_route(
    db: State<'_, Database>,
    body: Ruma<get_profile::Request>,
) -> ConduitResult<get_profile::Response> {
    let avatar_url = db.users.avatar_url(&body.user_id)?;
    let displayname = db.users.displayname(&body.user_id)?;

    if avatar_url.is_none() && displayname.is_none() {
        // Return 404 if we don't have a profile for this id
        return Err(Error::BadRequest(
            ErrorKind::NotFound,
            "Profile was not found.",
        ));
    }

    Ok(get_profile::Response {
        avatar_url,
        displayname,
    }
    .into())
}

#[cfg_attr(
    feature = "conduit_bin",
    put("/_matrix/client/r0/presence/<_>/status", data = "<body>")
)]
pub fn set_presence_route(
    db: State<'_, Database>,
    body: Ruma<set_presence::Request>,
) -> ConduitResult<set_presence::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");

    for room_id in db.rooms.rooms_joined(&sender_id) {
        let room_id = room_id?;

        db.rooms.edus.update_presence(
            &sender_id,
            &room_id,
            ruma::events::presence::PresenceEvent {
                content: ruma::events::presence::PresenceEventContent {
                    avatar_url: db.users.avatar_url(&sender_id)?,
                    currently_active: None,
                    displayname: db.users.displayname(&sender_id)?,
                    last_active_ago: Some(
                        utils::millis_since_unix_epoch()
                            .try_into()
                            .expect("time is valid"),
                    ),
                    presence: body.presence,
                    status_msg: body.status_msg.clone(),
                },
                sender: sender_id.clone(),
            },
            &db.globals,
        )?;
    }

    Ok(set_presence::Response.into())
}

#[cfg_attr(
    feature = "conduit_bin",
    post("/_matrix/client/r0/keys/upload", data = "<body>")
)]
pub fn upload_keys_route(
    db: State<'_, Database>,
    body: Ruma<upload_keys::Request>,
) -> ConduitResult<upload_keys::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");
    let device_id = body.device_id.as_ref().expect("user is authenticated");

    if let Some(one_time_keys) = &body.one_time_keys {
        for (key_key, key_value) in one_time_keys {
            db.users
                .add_one_time_key(sender_id, device_id, key_key, key_value)?;
        }
    }

    if let Some(device_keys) = &body.device_keys {
        // This check is needed to assure that signatures are kept
        if db.users.get_device_keys(sender_id, device_id)?.is_none() {
            db.users
                .add_device_keys(sender_id, device_id, device_keys, &db.rooms, &db.globals)?;
        }
    }

    Ok(upload_keys::Response {
        one_time_key_counts: db.users.count_one_time_keys(sender_id, device_id)?,
    }
    .into())
}

#[cfg_attr(
    feature = "conduit_bin",
    post("/_matrix/client/r0/keys/query", data = "<body>")
)]
pub fn get_keys_route(
    db: State<'_, Database>,
    body: Ruma<get_keys::Request>,
) -> ConduitResult<get_keys::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");

    let mut master_keys = BTreeMap::new();
    let mut self_signing_keys = BTreeMap::new();
    let mut user_signing_keys = BTreeMap::new();
    let mut device_keys = BTreeMap::new();

    for (user_id, device_ids) in &body.device_keys {
        if device_ids.is_empty() {
            let mut container = BTreeMap::new();
            for device_id in db.users.all_device_ids(user_id) {
                let device_id = device_id?;
                if let Some(mut keys) = db.users.get_device_keys(user_id, &device_id)? {
                    let metadata = db
                        .users
                        .get_device_metadata(user_id, &device_id)?
                        .ok_or_else(|| {
                            Error::bad_database("all_device_keys contained nonexistent device.")
                        })?;

                    keys.unsigned = Some(keys::UnsignedDeviceInfo {
                        device_display_name: metadata.display_name,
                    });

                    container.insert(device_id, keys);
                }
            }
            device_keys.insert(user_id.clone(), container);
        } else {
            for device_id in device_ids {
                let mut container = BTreeMap::new();
                if let Some(mut keys) = db.users.get_device_keys(&user_id.clone(), &device_id)? {
                    let metadata = db.users.get_device_metadata(user_id, &device_id)?.ok_or(
                        Error::BadRequest(
                            ErrorKind::InvalidParam,
                            "Tried to get keys for nonexistent device.",
                        ),
                    )?;

                    keys.unsigned = Some(keys::UnsignedDeviceInfo {
                        device_display_name: metadata.display_name,
                    });

                    container.insert(device_id.clone(), keys);
                }
                device_keys.insert(user_id.clone(), container);
            }
        }

        if let Some(master_key) = db.users.get_master_key(user_id, sender_id)? {
            master_keys.insert(user_id.clone(), master_key);
        }
        if let Some(self_signing_key) = db.users.get_self_signing_key(user_id, sender_id)? {
            self_signing_keys.insert(user_id.clone(), self_signing_key);
        }
        if user_id == sender_id {
            if let Some(user_signing_key) = db.users.get_user_signing_key(sender_id)? {
                user_signing_keys.insert(user_id.clone(), user_signing_key);
            }
        }
    }

    Ok(get_keys::Response {
        master_keys,
        self_signing_keys,
        user_signing_keys,
        device_keys,
        failures: BTreeMap::new(),
    }
    .into())
}

#[cfg_attr(
    feature = "conduit_bin",
    post("/_matrix/client/r0/keys/claim", data = "<body>")
)]
pub fn claim_keys_route(
    db: State<'_, Database>,
    body: Ruma<claim_keys::Request>,
) -> ConduitResult<claim_keys::Response> {
    let mut one_time_keys = BTreeMap::new();
    for (user_id, map) in &body.one_time_keys {
        let mut container = BTreeMap::new();
        for (device_id, key_algorithm) in map {
            if let Some(one_time_keys) =
                db.users
                    .take_one_time_key(user_id, device_id, key_algorithm)?
            {
                let mut c = BTreeMap::new();
                c.insert(one_time_keys.0, one_time_keys.1);
                container.insert(device_id.clone(), c);
            }
        }
        one_time_keys.insert(user_id.clone(), container);
    }

    Ok(claim_keys::Response {
        failures: BTreeMap::new(),
        one_time_keys,
    }
    .into())
}

#[cfg_attr(
    feature = "conduit_bin",
    post("/_matrix/client/unstable/room_keys/version", data = "<body>")
)]
pub fn create_backup_route(
    db: State<'_, Database>,
    body: Ruma<create_backup::Request>,
) -> ConduitResult<create_backup::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");
    let version = db
        .key_backups
        .create_backup(&sender_id, &body.algorithm, &db.globals)?;

    Ok(create_backup::Response { version }.into())
}

#[cfg_attr(
    feature = "conduit_bin",
    put("/_matrix/client/unstable/room_keys/version/<_>", data = "<body>")
)]
pub fn update_backup_route(
    db: State<'_, Database>,
    body: Ruma<update_backup::Request>,
) -> ConduitResult<update_backup::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");
    db.key_backups
        .update_backup(&sender_id, &body.version, &body.algorithm, &db.globals)?;

    Ok(update_backup::Response.into())
}

#[cfg_attr(
    feature = "conduit_bin",
    get("/_matrix/client/unstable/room_keys/version", data = "<body>")
)]
pub fn get_latest_backup_route(
    db: State<'_, Database>,
    body: Ruma<get_latest_backup::Request>,
) -> ConduitResult<get_latest_backup::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");

    let (version, algorithm) =
        db.key_backups
            .get_latest_backup(&sender_id)?
            .ok_or(Error::BadRequest(
                ErrorKind::NotFound,
                "Key backup does not exist.",
            ))?;

    Ok(get_latest_backup::Response {
        algorithm,
        count: (db.key_backups.count_keys(sender_id, &version)? as u32).into(),
        etag: db.key_backups.get_etag(sender_id, &version)?,
        version,
    }
    .into())
}

#[cfg_attr(
    feature = "conduit_bin",
    get("/_matrix/client/unstable/room_keys/version/<_>", data = "<body>")
)]
pub fn get_backup_route(
    db: State<'_, Database>,
    body: Ruma<get_backup::Request>,
) -> ConduitResult<get_backup::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");
    let algorithm = db
        .key_backups
        .get_backup(&sender_id, &body.version)?
        .ok_or(Error::BadRequest(
            ErrorKind::NotFound,
            "Key backup does not exist.",
        ))?;

    Ok(get_backup::Response {
        algorithm,
        count: (db.key_backups.count_keys(sender_id, &body.version)? as u32).into(),
        etag: db.key_backups.get_etag(sender_id, &body.version)?,
        version: body.version.clone(),
    }
    .into())
}

/// Add the received backup_keys to the database.
#[cfg_attr(
    feature = "conduit_bin",
    put("/_matrix/client/unstable/room_keys/keys", data = "<body>")
)]
pub fn add_backup_keys_route(
    db: State<'_, Database>,
    body: Ruma<add_backup_keys::Request>,
) -> ConduitResult<add_backup_keys::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");

    for (room_id, room) in &body.rooms {
        for (session_id, key_data) in &room.sessions {
            db.key_backups.add_key(
                &sender_id,
                &body.version,
                &room_id,
                &session_id,
                &key_data,
                &db.globals,
            )?
        }
    }

    Ok(add_backup_keys::Response {
        count: (db.key_backups.count_keys(sender_id, &body.version)? as u32).into(),
        etag: db.key_backups.get_etag(sender_id, &body.version)?,
    }
    .into())
}

#[cfg_attr(
    feature = "conduit_bin",
    get("/_matrix/client/unstable/room_keys/keys", data = "<body>")
)]
pub fn get_backup_keys_route(
    db: State<'_, Database>,
    body: Ruma<get_backup_keys::Request>,
) -> ConduitResult<get_backup_keys::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");

    let rooms = db.key_backups.get_all(&sender_id, &body.version)?;

    Ok(get_backup_keys::Response { rooms }.into())
}

#[cfg_attr(
    feature = "conduit_bin",
    post("/_matrix/client/r0/rooms/<_>/read_markers", data = "<body>")
)]
pub fn set_read_marker_route(
    db: State<'_, Database>,
    body: Ruma<set_read_marker::Request>,
) -> ConduitResult<set_read_marker::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");

    let fully_read_event = ruma::events::fully_read::FullyReadEvent {
        content: ruma::events::fully_read::FullyReadEventContent {
            event_id: body.fully_read.clone(),
        },
        room_id: body.room_id.clone(),
    };
    db.account_data.update(
        Some(&body.room_id),
        &sender_id,
        EventType::FullyRead,
        &fully_read_event,
        &db.globals,
    )?;

    if let Some(event) = &body.read_receipt {
        db.rooms.edus.room_read_set(
            &body.room_id,
            &sender_id,
            db.rooms.get_pdu_count(event)?.ok_or(Error::BadRequest(
                ErrorKind::InvalidParam,
                "Event does not exist.",
            ))?,
        )?;

        let mut user_receipts = BTreeMap::new();
        user_receipts.insert(
            sender_id.clone(),
            ruma::events::receipt::Receipt {
                ts: Some(SystemTime::now()),
            },
        );
        let mut receipt_content = BTreeMap::new();
        receipt_content.insert(
            event.clone(),
            ruma::events::receipt::Receipts {
                read: Some(user_receipts),
            },
        );

        db.rooms.edus.roomlatest_update(
            &sender_id,
            &body.room_id,
            AnyEvent::Ephemeral(AnyEphemeralRoomEvent::Receipt(
                ruma::events::receipt::ReceiptEvent {
                    content: ruma::events::receipt::ReceiptEventContent(receipt_content),
                    room_id: body.room_id.clone(),
                },
            )),
            &db.globals,
        )?;
    }
    Ok(set_read_marker::Response.into())
}

#[cfg_attr(
    feature = "conduit_bin",
    put("/_matrix/client/r0/rooms/<_>/typing/<_>", data = "<body>")
)]
pub fn create_typing_event_route(
    db: State<'_, Database>,
    body: Ruma<create_typing_event::Request>,
) -> ConduitResult<create_typing_event::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");

    if body.typing {
        db.rooms.edus.roomactive_add(
            &sender_id,
            &body.room_id,
            body.timeout.map(|d| d.as_millis() as u64).unwrap_or(30000)
                + utils::millis_since_unix_epoch(),
            &db.globals,
        )?;
    } else {
        db.rooms
            .edus
            .roomactive_remove(&sender_id, &body.room_id, &db.globals)?;
    }

    Ok(create_typing_event::Response.into())
}

#[cfg_attr(
    feature = "conduit_bin",
    post("/_matrix/client/r0/createRoom", data = "<body>")
)]
pub fn create_room_route(
    db: State<'_, Database>,
    body: Ruma<create_room::Request>,
) -> ConduitResult<create_room::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");

    let room_id = RoomId::new(db.globals.server_name());

    let alias = body
        .room_alias_name
        .as_ref()
        .map_or(Ok(None), |localpart| {
            // TODO: Check for invalid characters and maximum length
            let alias =
                RoomAliasId::try_from(format!("#{}:{}", localpart, db.globals.server_name()))
                    .map_err(|_| Error::BadRequest(ErrorKind::InvalidParam, "Invalid alias."))?;

            if db.rooms.id_from_alias(&alias)?.is_some() {
                Err(Error::BadRequest(
                    ErrorKind::RoomInUse,
                    "Room alias already exists.",
                ))
            } else {
                Ok(Some(alias))
            }
        })?;

    let mut content = ruma::events::room::create::CreateEventContent::new(sender_id.clone());
    content.federate = body.creation_content.as_ref().map_or(true, |c| c.federate);
    content.predecessor = body
        .creation_content
        .as_ref()
        .and_then(|c| c.predecessor.clone());
    content.room_version = RoomVersionId::Version6;

    // 1. The room create event
    db.rooms.append_pdu(
        room_id.clone(),
        sender_id.clone(),
        EventType::RoomCreate,
        serde_json::to_value(content).expect("event is valid, we just created it"),
        None,
        Some("".to_owned()),
        None,
        &db.globals,
    )?;

    // 2. Let the room creator join
    db.rooms.append_pdu(
        room_id.clone(),
        sender_id.clone(),
        EventType::RoomMember,
        serde_json::to_value(member::MemberEventContent {
            membership: member::MembershipState::Join,
            displayname: db.users.displayname(&sender_id)?,
            avatar_url: db.users.avatar_url(&sender_id)?,
            is_direct: body.is_direct,
            third_party_invite: None,
        })
        .expect("event is valid, we just created it"),
        None,
        Some(sender_id.to_string()),
        None,
        &db.globals,
    )?;

    // Figure out preset. We need it for power levels and preset specific events
    let visibility = body.visibility.unwrap_or(room::Visibility::Private);
    let preset = body.preset.unwrap_or_else(|| match visibility {
        room::Visibility::Private => create_room::RoomPreset::PrivateChat,
        room::Visibility::Public => create_room::RoomPreset::PublicChat,
    });

    // 3. Power levels
    let mut users = BTreeMap::new();
    users.insert(sender_id.clone(), 100.into());
    for invite_ in &body.invite {
        users.insert(invite_.clone(), 100.into());
    }

    let power_levels_content = if let Some(power_levels) = &body.power_level_content_override {
        serde_json::from_str(power_levels.json().get()).map_err(|_| {
            Error::BadRequest(ErrorKind::BadJson, "Invalid power_level_content_override.")
        })?
    } else {
        serde_json::to_value(ruma::events::room::power_levels::PowerLevelsEventContent {
            ban: 50.into(),
            events: BTreeMap::new(),
            events_default: 0.into(),
            invite: 50.into(),
            kick: 50.into(),
            redact: 50.into(),
            state_default: 50.into(),
            users,
            users_default: 0.into(),
            notifications: ruma::events::room::power_levels::NotificationPowerLevels {
                room: 50.into(),
            },
        })
        .expect("event is valid, we just created it")
    };
    db.rooms.append_pdu(
        room_id.clone(),
        sender_id.clone(),
        EventType::RoomPowerLevels,
        power_levels_content,
        None,
        Some("".to_owned()),
        None,
        &db.globals,
    )?;

    // 4. Events set by preset
    // 4.1 Join Rules
    db.rooms.append_pdu(
        room_id.clone(),
        sender_id.clone(),
        EventType::RoomJoinRules,
        match preset {
            create_room::RoomPreset::PublicChat => serde_json::to_value(
                join_rules::JoinRulesEventContent::new(join_rules::JoinRule::Public),
            )
            .expect("event is valid, we just created it"),
            // according to spec "invite" is the default
            _ => serde_json::to_value(join_rules::JoinRulesEventContent::new(
                join_rules::JoinRule::Invite,
            ))
            .expect("event is valid, we just created it"),
        },
        None,
        Some("".to_owned()),
        None,
        &db.globals,
    )?;

    // 4.2 History Visibility
    db.rooms.append_pdu(
        room_id.clone(),
        sender_id.clone(),
        EventType::RoomHistoryVisibility,
        serde_json::to_value(history_visibility::HistoryVisibilityEventContent::new(
            history_visibility::HistoryVisibility::Shared,
        ))
        .expect("event is valid, we just created it"),
        None,
        Some("".to_owned()),
        None,
        &db.globals,
    )?;

    // 4.3 Guest Access
    db.rooms.append_pdu(
        room_id.clone(),
        sender_id.clone(),
        EventType::RoomGuestAccess,
        match preset {
            create_room::RoomPreset::PublicChat => serde_json::to_value(
                guest_access::GuestAccessEventContent::new(guest_access::GuestAccess::Forbidden),
            )
            .expect("event is valid, we just created it"),
            _ => serde_json::to_value(guest_access::GuestAccessEventContent::new(
                guest_access::GuestAccess::CanJoin,
            ))
            .expect("event is valid, we just created it"),
        },
        None,
        Some("".to_owned()),
        None,
        &db.globals,
    )?;

    // 5. Events listed in initial_state
    for create_room::InitialStateEvent {
        event_type,
        state_key,
        content,
    } in &body.initial_state
    {
        // Silently skip encryption events if they are not allowed
        if event_type == &EventType::RoomEncryption && db.globals.encryption_disabled() {
            continue;
        }

        db.rooms.append_pdu(
            room_id.clone(),
            sender_id.clone(),
            event_type.clone(),
            serde_json::from_str(content.get()).map_err(|_| {
                Error::BadRequest(ErrorKind::BadJson, "Invalid initial_state content.")
            })?,
            None,
            state_key.clone(),
            None,
            &db.globals,
        )?;
    }

    // 6. Events implied by name and topic
    if let Some(name) = &body.name {
        db.rooms.append_pdu(
            room_id.clone(),
            sender_id.clone(),
            EventType::RoomName,
            serde_json::to_value(
                name::NameEventContent::new(name.clone())
                    .map_err(|_| Error::BadRequest(ErrorKind::InvalidParam, "Name is invalid."))?,
            )
            .expect("event is valid, we just created it"),
            None,
            Some("".to_owned()),
            None,
            &db.globals,
        )?;
    }

    if let Some(topic) = &body.topic {
        db.rooms.append_pdu(
            room_id.clone(),
            sender_id.clone(),
            EventType::RoomTopic,
            serde_json::to_value(topic::TopicEventContent {
                topic: topic.clone(),
            })
            .expect("event is valid, we just created it"),
            None,
            Some("".to_owned()),
            None,
            &db.globals,
        )?;
    }

    // 7. Events implied by invite (and TODO: invite_3pid)
    for user in &body.invite {
        db.rooms.append_pdu(
            room_id.clone(),
            sender_id.clone(),
            EventType::RoomMember,
            serde_json::to_value(member::MemberEventContent {
                membership: member::MembershipState::Invite,
                displayname: db.users.displayname(&user)?,
                avatar_url: db.users.avatar_url(&user)?,
                is_direct: body.is_direct,
                third_party_invite: None,
            })
            .expect("event is valid, we just created it"),
            None,
            Some(user.to_string()),
            None,
            &db.globals,
        )?;
    }

    // Homeserver specific stuff
    if let Some(alias) = alias {
        db.rooms.set_alias(&alias, Some(&room_id), &db.globals)?;
    }

    if let Some(room::Visibility::Public) = body.visibility {
        db.rooms.set_public(&room_id, true)?;
    }

    Ok(create_room::Response { room_id }.into())
}

#[cfg_attr(
    feature = "conduit_bin",
    get("/_matrix/client/r0/joined_rooms", data = "<body>")
)]
pub fn joined_rooms_route(
    db: State<'_, Database>,
    body: Ruma<joined_rooms::Request>,
) -> ConduitResult<joined_rooms::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");

    Ok(joined_rooms::Response {
        joined_rooms: db
            .rooms
            .rooms_joined(&sender_id)
            .filter_map(|r| r.ok())
            .collect(),
    }
    .into())
}

#[cfg_attr(
    feature = "conduit_bin",
    put("/_matrix/client/r0/rooms/<_>/redact/<_>/<_>", data = "<body>")
)]
pub fn redact_event_route(
    db: State<'_, Database>,
    body: Ruma<redact_event::Request>,
) -> ConduitResult<redact_event::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");

    let event_id = db.rooms.append_pdu(
        body.room_id.clone(),
        sender_id.clone(),
        EventType::RoomRedaction,
        serde_json::to_value(redaction::RedactionEventContent {
            reason: body.reason.clone(),
        })
        .expect("event is valid, we just created it"),
        None,
        None,
        Some(body.event_id.clone()),
        &db.globals,
    )?;

    Ok(redact_event::Response { event_id }.into())
}

#[cfg_attr(
    feature = "conduit_bin",
    put("/_matrix/client/r0/directory/room/<_>", data = "<body>")
)]
pub fn create_alias_route(
    db: State<'_, Database>,
    body: Ruma<create_alias::Request>,
) -> ConduitResult<create_alias::Response> {
    if db.rooms.id_from_alias(&body.room_alias)?.is_some() {
        return Err(Error::Conflict("Alias already exists."));
    }

    db.rooms
        .set_alias(&body.room_alias, Some(&body.room_id), &db.globals)?;

    Ok(create_alias::Response.into())
}

#[cfg_attr(
    feature = "conduit_bin",
    delete("/_matrix/client/r0/directory/room/<_>", data = "<body>")
)]
pub fn delete_alias_route(
    db: State<'_, Database>,
    body: Ruma<delete_alias::Request>,
) -> ConduitResult<delete_alias::Response> {
    db.rooms.set_alias(&body.room_alias, None, &db.globals)?;

    Ok(delete_alias::Response.into())
}

#[cfg_attr(
    feature = "conduit_bin",
    get("/_matrix/client/r0/directory/room/<_>", data = "<body>")
)]
pub fn get_alias_route(
    db: State<'_, Database>,
    body: Ruma<get_alias::Request>,
) -> ConduitResult<get_alias::Response> {
    if body.room_alias.server_name() != db.globals.server_name() {
        todo!("ask remote server");
    }

    let room_id = db
        .rooms
        .id_from_alias(&body.room_alias)?
        .ok_or(Error::BadRequest(
            ErrorKind::NotFound,
            "Room with alias not found.",
        ))?;

    Ok(get_alias::Response {
        room_id,
        servers: vec![db.globals.server_name().to_string()],
    }
    .into())
}

#[cfg_attr(
    feature = "conduit_bin",
    post("/_matrix/client/r0/rooms/<_>/join", data = "<body>")
)]
pub fn join_room_by_id_route(
    db: State<'_, Database>,
    body: Ruma<join_room_by_id::Request>,
) -> ConduitResult<join_room_by_id::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");

    // TODO: Ask a remote server if we don't have this room

    let event = member::MemberEventContent {
        membership: member::MembershipState::Join,
        displayname: db.users.displayname(&sender_id)?,
        avatar_url: db.users.avatar_url(&sender_id)?,
        is_direct: None,
        third_party_invite: None,
    };

    db.rooms.append_pdu(
        body.room_id.clone(),
        sender_id.clone(),
        EventType::RoomMember,
        serde_json::to_value(event).expect("event is valid, we just created it"),
        None,
        Some(sender_id.to_string()),
        None,
        &db.globals,
    )?;

    Ok(join_room_by_id::Response {
        room_id: body.room_id.clone(),
    }
    .into())
}

#[cfg_attr(
    feature = "conduit_bin",
    post("/_matrix/client/r0/join/<_>", data = "<body>")
)]
pub fn join_room_by_id_or_alias_route(
    db: State<'_, Database>,
    body: Ruma<join_room_by_id_or_alias::Request>,
) -> ConduitResult<join_room_by_id_or_alias::Response> {
    let room_id = RoomId::try_from(body.room_id_or_alias.clone()).or_else(|alias| {
        Ok::<_, Error>(db.rooms.id_from_alias(&alias)?.ok_or(Error::BadRequest(
            ErrorKind::NotFound,
            "Room not found (TODO: Federation).",
        ))?)
    })?;

    let body = Ruma {
        sender_id: body.sender_id.clone(),
        device_id: body.device_id.clone(),
        json_body: None,
        body: join_room_by_id::Request {
            room_id,
            third_party_signed: body.third_party_signed.clone(),
        },
    };

    Ok(join_room_by_id_or_alias::Response {
        room_id: join_room_by_id_route(db, body)?.0.room_id,
    }
    .into())
}

#[cfg_attr(
    feature = "conduit_bin",
    post("/_matrix/client/r0/rooms/<_>/leave", data = "<body>")
)]
pub fn leave_room_route(
    db: State<'_, Database>,
    body: Ruma<leave_room::Request>,
) -> ConduitResult<leave_room::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");

    let mut event = serde_json::from_value::<Raw<member::MemberEventContent>>(
        db.rooms
            .room_state_get(
                &body.room_id,
                &EventType::RoomMember,
                &sender_id.to_string(),
            )?
            .ok_or(Error::BadRequest(
                ErrorKind::BadState,
                "Cannot leave a room you are not a member of.",
            ))?
            .content
            .clone(),
    )
    .map_err(|_| Error::bad_database("Invalid member event in database."))?
    .deserialize()
    .map_err(|_| Error::bad_database("Invalid member event in database."))?;

    event.membership = member::MembershipState::Leave;

    db.rooms.append_pdu(
        body.room_id.clone(),
        sender_id.clone(),
        EventType::RoomMember,
        serde_json::to_value(event).expect("event is valid, we just created it"),
        None,
        Some(sender_id.to_string()),
        None,
        &db.globals,
    )?;

    Ok(leave_room::Response.into())
}

#[cfg_attr(
    feature = "conduit_bin",
    post("/_matrix/client/r0/rooms/<_>/kick", data = "<body>")
)]
pub fn kick_user_route(
    db: State<'_, Database>,
    body: Ruma<kick_user::Request>,
) -> ConduitResult<kick_user::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");

    let mut event = serde_json::from_value::<Raw<ruma::events::room::member::MemberEventContent>>(
        db.rooms
            .room_state_get(
                &body.room_id,
                &EventType::RoomMember,
                &body.user_id.to_string(),
            )?
            .ok_or(Error::BadRequest(
                ErrorKind::BadState,
                "Cannot kick member that's not in the room.",
            ))?
            .content
            .clone(),
    )
    .expect("Raw::from_value always works")
    .deserialize()
    .map_err(|_| Error::bad_database("Invalid member event in database."))?;

    event.membership = ruma::events::room::member::MembershipState::Leave;
    // TODO: reason

    db.rooms.append_pdu(
        body.room_id.clone(),
        sender_id.clone(),
        EventType::RoomMember,
        serde_json::to_value(event).expect("event is valid, we just created it"),
        None,
        Some(body.user_id.to_string()),
        None,
        &db.globals,
    )?;

    Ok(kick_user::Response.into())
}

#[cfg_attr(
    feature = "conduit_bin",
    get("/_matrix/client/r0/rooms/<_>/joined_members", data = "<body>")
)]
pub fn joined_members_route(
    db: State<'_, Database>,
    body: Ruma<joined_members::Request>,
) -> ConduitResult<joined_members::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");

    if !db
        .rooms
        .is_joined(&sender_id, &body.room_id)
        .unwrap_or(false)
    {
        return Err(Error::BadRequest(
            ErrorKind::Forbidden,
            "You aren't a member of the room.",
        ));
    }

    let mut joined = BTreeMap::new();
    for user_id in db.rooms.room_members(&body.room_id).filter_map(|r| r.ok()) {
        let display_name = db.users.displayname(&user_id)?;
        let avatar_url = db.users.avatar_url(&user_id)?;

        joined.insert(
            user_id,
            joined_members::RoomMember {
                display_name,
                avatar_url,
            },
        );
    }

    Ok(joined_members::Response { joined }.into())
}

#[cfg_attr(
    feature = "conduit_bin",
    post("/_matrix/client/r0/rooms/<_>/ban", data = "<body>")
)]
pub fn ban_user_route(
    db: State<'_, Database>,
    body: Ruma<ban_user::Request>,
) -> ConduitResult<ban_user::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");

    // TODO: reason

    let event = db
        .rooms
        .room_state_get(
            &body.room_id,
            &EventType::RoomMember,
            &body.user_id.to_string(),
        )?
        .map_or(
            Ok::<_, Error>(member::MemberEventContent {
                membership: member::MembershipState::Ban,
                displayname: db.users.displayname(&body.user_id)?,
                avatar_url: db.users.avatar_url(&body.user_id)?,
                is_direct: None,
                third_party_invite: None,
            }),
            |event| {
                let mut event = serde_json::from_value::<Raw<member::MemberEventContent>>(
                    event.content.clone(),
                )
                .expect("Raw::from_value always works")
                .deserialize()
                .map_err(|_| Error::bad_database("Invalid member event in database."))?;
                event.membership = ruma::events::room::member::MembershipState::Ban;
                Ok(event)
            },
        )?;

    db.rooms.append_pdu(
        body.room_id.clone(),
        sender_id.clone(),
        EventType::RoomMember,
        serde_json::to_value(event).expect("event is valid, we just created it"),
        None,
        Some(body.user_id.to_string()),
        None,
        &db.globals,
    )?;

    Ok(ban_user::Response.into())
}

#[cfg_attr(
    feature = "conduit_bin",
    post("/_matrix/client/r0/rooms/<_>/unban", data = "<body>")
)]
pub fn unban_user_route(
    db: State<'_, Database>,
    body: Ruma<unban_user::Request>,
) -> ConduitResult<unban_user::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");

    let mut event = serde_json::from_value::<Raw<ruma::events::room::member::MemberEventContent>>(
        db.rooms
            .room_state_get(
                &body.room_id,
                &EventType::RoomMember,
                &body.user_id.to_string(),
            )?
            .ok_or(Error::BadRequest(
                ErrorKind::BadState,
                "Cannot unban a user who is not banned.",
            ))?
            .content
            .clone(),
    )
    .map_err(|_| Error::bad_database("Invalid member event in database."))?
    .deserialize()
    .map_err(|_| Error::bad_database("Invalid member event in database."))?;

    event.membership = ruma::events::room::member::MembershipState::Leave;

    db.rooms.append_pdu(
        body.room_id.clone(),
        sender_id.clone(),
        EventType::RoomMember,
        serde_json::to_value(event).expect("event is valid, we just created it"),
        None,
        Some(body.user_id.to_string()),
        None,
        &db.globals,
    )?;

    Ok(unban_user::Response.into())
}

#[cfg_attr(
    feature = "conduit_bin",
    post("/_matrix/client/r0/rooms/<_>/forget", data = "<body>")
)]
pub fn forget_room_route(
    db: State<'_, Database>,
    body: Ruma<forget_room::Request>,
) -> ConduitResult<forget_room::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");

    db.rooms.forget(&body.room_id, &sender_id)?;

    Ok(forget_room::Response.into())
}

#[cfg_attr(
    feature = "conduit_bin",
    post("/_matrix/client/r0/rooms/<_>/invite", data = "<body>")
)]
pub fn invite_user_route(
    db: State<'_, Database>,
    body: Ruma<invite_user::Request>,
) -> ConduitResult<invite_user::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");

    if let invite_user::InvitationRecipient::UserId { user_id } = &body.recipient {
        db.rooms.append_pdu(
            body.room_id.clone(),
            sender_id.clone(),
            EventType::RoomMember,
            serde_json::to_value(member::MemberEventContent {
                membership: member::MembershipState::Invite,
                displayname: db.users.displayname(&user_id)?,
                avatar_url: db.users.avatar_url(&user_id)?,
                is_direct: None,
                third_party_invite: None,
            })
            .expect("event is valid, we just created it"),
            None,
            Some(user_id.to_string()),
            None,
            &db.globals,
        )?;

        Ok(invite_user::Response.into())
    } else {
        Err(Error::BadRequest(ErrorKind::NotFound, "User not found."))
    }
}

#[cfg_attr(
    feature = "conduit_bin",
    put("/_matrix/client/r0/directory/list/room/<_>", data = "<body>")
)]
pub async fn set_room_visibility_route(
    db: State<'_, Database>,
    body: Ruma<set_room_visibility::Request>,
) -> ConduitResult<set_room_visibility::Response> {
    match body.visibility {
        room::Visibility::Public => db.rooms.set_public(&body.room_id, true)?,
        room::Visibility::Private => db.rooms.set_public(&body.room_id, false)?,
    }

    Ok(set_room_visibility::Response.into())
}

#[cfg_attr(
    feature = "conduit_bin",
    get("/_matrix/client/r0/directory/list/room/<_>", data = "<body>")
)]
pub async fn get_room_visibility_route(
    db: State<'_, Database>,
    body: Ruma<get_room_visibility::Request>,
) -> ConduitResult<get_room_visibility::Response> {
    Ok(get_room_visibility::Response {
        visibility: if db.rooms.is_public_room(&body.room_id)? {
            room::Visibility::Public
        } else {
            room::Visibility::Private
        },
    }
    .into())
}

#[cfg_attr(
    feature = "conduit_bin",
    get("/_matrix/client/r0/publicRooms", data = "<body>")
)]
pub async fn get_public_rooms_route(
    db: State<'_, Database>,
    body: Ruma<get_public_rooms::Request>,
) -> ConduitResult<get_public_rooms::Response> {
    let Ruma {
        body:
            get_public_rooms::Request {
                limit,
                server,
                since,
            },
        sender_id,
        device_id,
        json_body,
    } = body;

    let get_public_rooms_filtered::Response {
        chunk,
        prev_batch,
        next_batch,
        total_room_count_estimate,
    } = get_public_rooms_filtered_route(
        db,
        Ruma {
            body: get_public_rooms_filtered::Request {
                filter: None,
                limit,
                room_network: get_public_rooms_filtered::RoomNetwork::Matrix,
                server,
                since,
            },
            sender_id,
            device_id,
            json_body,
        },
    )
    .await?
    .0;

    Ok(get_public_rooms::Response {
        chunk,
        prev_batch,
        next_batch,
        total_room_count_estimate,
    }
    .into())
}

#[cfg_attr(
    feature = "conduit_bin",
    post("/_matrix/client/r0/publicRooms", data = "<_body>")
)]
pub async fn get_public_rooms_filtered_route(
    db: State<'_, Database>,
    _body: Ruma<get_public_rooms_filtered::Request>,
) -> ConduitResult<get_public_rooms_filtered::Response> {
    let mut chunk = db
        .rooms
        .public_rooms()
        .map(|room_id| {
            let room_id = room_id?;

            // TODO: Do not load full state?
            let state = db.rooms.room_state_full(&room_id)?;

            let chunk = directory::PublicRoomsChunk {
                aliases: Vec::new(),
                canonical_alias: state.get(&(EventType::RoomCanonicalAlias, "".to_owned())).map_or(Ok::<_, Error>(None), |s| {
                    Ok(serde_json::from_value::<
                            Raw<ruma::events::room::canonical_alias::CanonicalAliasEventContent>,
                        >(s.content.clone())
                        .map_err(|_| Error::bad_database("Invalid canonical alias event in database."))?
                        .deserialize()
                        .map_err(|_| Error::bad_database("Invalid canonical alias event in database."))?
                        .alias)
                })?,
                name: state.get(&(EventType::RoomName, "".to_owned())).map_or(Ok::<_, Error>(None), |s| {
                    Ok(serde_json::from_value::<Raw<ruma::events::room::name::NameEventContent>>(
                        s.content.clone(),
                    )
                    .map_err(|_| Error::bad_database("Invalid room name event in database."))?
                    .deserialize()
                    .map_err(|_| Error::bad_database("Invalid room name event in database."))?
                    .name()
                    .map(|n| n.to_owned()))
                })?,
                num_joined_members: (db.rooms.room_members(&room_id).count() as u32).into(),
                room_id,
                topic: state.get(&(EventType::RoomTopic, "".to_owned())).map_or(Ok::<_, Error>(None), |s| {
                    Ok(Some(serde_json::from_value::<
                            Raw<ruma::events::room::topic::TopicEventContent>,
                        >(s.content.clone())
                        .map_err(|_| Error::bad_database("Invalid room topic event in database."))?
                        .deserialize()
                        .map_err(|_| Error::bad_database("Invalid room topic event in database."))?
                        .topic))
                })?,
                world_readable: state.get(&(EventType::RoomHistoryVisibility, "".to_owned())).map_or(Ok::<_, Error>(false), |s| {
                    Ok(serde_json::from_value::<
                            Raw<ruma::events::room::history_visibility::HistoryVisibilityEventContent>,
                        >(s.content.clone())
                        .map_err(|_| Error::bad_database("Invalid room history visibility event in database."))?
                        .deserialize()
                        .map_err(|_| Error::bad_database("Invalid room history visibility event in database."))?
                        .history_visibility == history_visibility::HistoryVisibility::WorldReadable)
                })?,
                guest_can_join: state.get(&(EventType::RoomGuestAccess, "".to_owned())).map_or(Ok::<_, Error>(false), |s| {
                    Ok(serde_json::from_value::<
                            Raw<ruma::events::room::guest_access::GuestAccessEventContent>,
                        >(s.content.clone())
                        .map_err(|_| Error::bad_database("Invalid room guest access event in database."))?
                        .deserialize()
                        .map_err(|_| Error::bad_database("Invalid room guest access event in database."))?
                        .guest_access == guest_access::GuestAccess::CanJoin)
                })?,
                avatar_url: state.get(&(EventType::RoomAvatar, "".to_owned())).map_or( Ok::<_, Error>(None),|s| {
                    Ok(Some(serde_json::from_value::<
                            Raw<ruma::events::room::avatar::AvatarEventContent>,
                        >(s.content.clone())
                        .map_err(|_| Error::bad_database("Invalid room avatar event in database."))?
                        .deserialize()
                        .map_err(|_| Error::bad_database("Invalid room avatar event in database."))?
                        .url))
                })?,
            };
            Ok::<_, Error>(chunk)
        })
        .filter_map(|r| r.ok()) // Filter out buggy rooms
        .collect::<Vec<_>>();

    chunk.sort_by(|l, r| r.num_joined_members.cmp(&l.num_joined_members));

    /*
    chunk.extend_from_slice(
        &server_server::send_request(
            &db,
            "privacytools.io".to_owned(),
            ruma::api::federation::v1::get_public_rooms::Request {
                limit: Some(20_u32.into()),
                since: None,
                room_network: ruma::api::federation::v1::get_public_rooms::RoomNetwork::Matrix,
            },
        )
        .await
        ?
        .chunk
        .into_iter()
        .map(|c| serde_json::from_str(&serde_json::to_string(&c)?)?)
        .collect::<Vec<_>>(),
    );
    */

    let total_room_count_estimate = (chunk.len() as u32).into();

    Ok(get_public_rooms_filtered::Response {
        chunk,
        prev_batch: None,
        next_batch: None,
        total_room_count_estimate: Some(total_room_count_estimate),
    }
    .into())
}

#[cfg_attr(
    feature = "conduit_bin",
    post("/_matrix/client/r0/user_directory/search", data = "<body>")
)]
pub fn search_users_route(
    db: State<'_, Database>,
    body: Ruma<search_users::Request>,
) -> ConduitResult<search_users::Response> {
    Ok(search_users::Response {
        results: db
            .users
            .iter()
            .filter_map(|user_id| {
                // Filter out buggy users (they should not exist, but you never know...)
                let user_id = user_id.ok()?;
                if db.users.is_deactivated(&user_id).ok()? {
                    return None;
                }

                let user = search_users::User {
                    user_id: user_id.clone(),
                    display_name: db.users.displayname(&user_id).ok()?,
                    avatar_url: db.users.avatar_url(&user_id).ok()?,
                };

                if !user.user_id.to_string().contains(&body.search_term)
                    && user
                        .display_name
                        .as_ref()
                        .filter(|name| name.contains(&body.search_term))
                        .is_none()
                {
                    return None;
                }

                Some(user)
            })
            .collect(),
        limited: false,
    }
    .into())
}

#[cfg_attr(
    feature = "conduit_bin",
    get("/_matrix/client/r0/rooms/<_>/members", data = "<body>")
)]
pub fn get_member_events_route(
    db: State<'_, Database>,
    body: Ruma<get_member_events::Request>,
) -> ConduitResult<get_member_events::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");

    if !db.rooms.is_joined(sender_id, &body.room_id)? {
        return Err(Error::BadRequest(
            ErrorKind::Forbidden,
            "You don't have permission to view this room.",
        ));
    }

    Ok(get_member_events::Response {
        chunk: db
            .rooms
            .room_state_type(&body.room_id, &EventType::RoomMember)?
            .values()
            .map(|pdu| pdu.to_member_event())
            .collect(),
    }
    .into())
}

#[cfg_attr(
    feature = "conduit_bin",
    get("/_matrix/client/r0/thirdparty/protocols")
)]
pub fn get_protocols_route() -> ConduitResult<get_protocols::Response> {
    warn!("TODO: get_protocols_route");
    Ok(get_protocols::Response {
        protocols: BTreeMap::new(),
    }
    .into())
}

#[cfg_attr(
    feature = "conduit_bin",
    get("/_matrix/client/r0/rooms/<_>/event/<_>", data = "<body>")
)]
pub fn get_room_event_route(
    db: State<'_, Database>,
    body: Ruma<get_room_event::Request>,
) -> ConduitResult<get_room_event::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");

    if !db.rooms.is_joined(sender_id, &body.room_id)? {
        return Err(Error::BadRequest(
            ErrorKind::Forbidden,
            "You don't have permission to view this room.",
        ));
    }

    Ok(get_room_event::Response {
        event: db
            .rooms
            .get_pdu(&body.event_id)?
            .ok_or(Error::BadRequest(ErrorKind::NotFound, "Event not found."))?
            .to_room_event(),
    }
    .into())
}

#[cfg_attr(
    feature = "conduit_bin",
    put("/_matrix/client/r0/rooms/<_>/send/<_>/<_>", data = "<body>")
)]
pub fn create_message_event_route(
    db: State<'_, Database>,
    body: Ruma<create_message_event::Request>,
) -> ConduitResult<create_message_event::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");

    let mut unsigned = serde_json::Map::new();
    unsigned.insert("transaction_id".to_owned(), body.txn_id.clone().into());

    let event_id = db.rooms.append_pdu(
        body.room_id.clone(),
        sender_id.clone(),
        body.event_type.clone(),
        serde_json::from_str(
            body.json_body
                .ok_or(Error::BadRequest(ErrorKind::BadJson, "Invalid JSON body."))?
                .get(),
        )
        .map_err(|_| Error::BadRequest(ErrorKind::BadJson, "Invalid JSON body."))?,
        Some(unsigned),
        None,
        None,
        &db.globals,
    )?;

    Ok(create_message_event::Response { event_id }.into())
}

#[cfg_attr(
    feature = "conduit_bin",
    put("/_matrix/client/r0/rooms/<_>/state/<_>/<_>", data = "<body>")
)]
pub fn create_state_event_for_key_route(
    db: State<'_, Database>,
    body: Ruma<create_state_event_for_key::Request>,
) -> ConduitResult<create_state_event_for_key::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");

    let content = serde_json::from_str::<serde_json::Value>(
        body.json_body
            .as_ref()
            .ok_or(Error::BadRequest(ErrorKind::BadJson, "Invalid JSON body."))?
            .get(),
    )
    .map_err(|_| Error::BadRequest(ErrorKind::BadJson, "Invalid JSON body."))?;

    if body.event_type == EventType::RoomCanonicalAlias {
        let canonical_alias = serde_json::from_value::<
            Raw<canonical_alias::CanonicalAliasEventContent>,
        >(content.clone())
        .map_err(|_| Error::BadRequest(ErrorKind::InvalidParam, "Invalid canonical alias."))?
        .deserialize()
        .map_err(|_| Error::BadRequest(ErrorKind::InvalidParam, "Invalid canonical alias."))?;

        let mut aliases = canonical_alias.alt_aliases;

        if let Some(alias) = canonical_alias.alias {
            aliases.push(alias);
        }

        for alias in aliases {
            if alias.server_name() != db.globals.server_name()
                || db
                    .rooms
                    .id_from_alias(&alias)?
                    .filter(|room| room == &body.room_id) // Make sure it's the right room
                    .is_none()
            {
                return Err(Error::BadRequest(ErrorKind::Forbidden, "You are only allowed to send canonical_alias events when it's aliases already exists"));
            }
        }
    }

    let event_id = db.rooms.append_pdu(
        body.room_id.clone(),
        sender_id.clone(),
        body.event_type.clone(),
        content,
        None,
        Some(body.state_key.clone()),
        None,
        &db.globals,
    )?;

    Ok(create_state_event_for_key::Response { event_id }.into())
}

#[cfg_attr(
    feature = "conduit_bin",
    put("/_matrix/client/r0/rooms/<_>/state/<_>", data = "<body>")
)]
pub fn create_state_event_for_empty_key_route(
    db: State<'_, Database>,
    body: Ruma<create_state_event_for_empty_key::Request>,
) -> ConduitResult<create_state_event_for_empty_key::Response> {
    // This just calls create_state_event_for_key_route
    let Ruma {
        body:
            create_state_event_for_empty_key::Request {
                room_id,
                event_type,
                data,
            },
        sender_id,
        device_id,
        json_body,
    } = body;

    Ok(create_state_event_for_empty_key::Response {
        event_id: create_state_event_for_key_route(
            db,
            Ruma {
                body: create_state_event_for_key::Request {
                    room_id,
                    event_type,
                    data,
                    state_key: "".to_owned(),
                },
                sender_id,
                device_id,
                json_body,
            },
        )?
        .0
        .event_id,
    }
    .into())
}

#[cfg_attr(
    feature = "conduit_bin",
    get("/_matrix/client/r0/rooms/<_>/state", data = "<body>")
)]
pub fn get_state_events_route(
    db: State<'_, Database>,
    body: Ruma<get_state_events::Request>,
) -> ConduitResult<get_state_events::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");

    if !db.rooms.is_joined(sender_id, &body.room_id)? {
        return Err(Error::BadRequest(
            ErrorKind::Forbidden,
            "You don't have permission to view the room state.",
        ));
    }

    Ok(get_state_events::Response {
        room_state: db
            .rooms
            .room_state_full(&body.room_id)?
            .values()
            .map(|pdu| pdu.to_state_event())
            .collect(),
    }
    .into())
}

#[cfg_attr(
    feature = "conduit_bin",
    get("/_matrix/client/r0/rooms/<_>/state/<_>/<_>", data = "<body>")
)]
pub fn get_state_events_for_key_route(
    db: State<'_, Database>,
    body: Ruma<get_state_events_for_key::Request>,
) -> ConduitResult<get_state_events_for_key::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");

    if !db.rooms.is_joined(sender_id, &body.room_id)? {
        return Err(Error::BadRequest(
            ErrorKind::Forbidden,
            "You don't have permission to view the room state.",
        ));
    }

    let event = db
        .rooms
        .room_state_get(&body.room_id, &body.event_type, &body.state_key)?
        .ok_or(Error::BadRequest(
            ErrorKind::NotFound,
            "State event not found.",
        ))?;

    Ok(get_state_events_for_key::Response {
        content: serde_json::value::to_raw_value(&event.content)
            .map_err(|_| Error::bad_database("Invalid event content in database"))?,
    }
    .into())
}

#[cfg_attr(
    feature = "conduit_bin",
    get("/_matrix/client/r0/rooms/<_>/state/<_>", data = "<body>")
)]
pub fn get_state_events_for_empty_key_route(
    db: State<'_, Database>,
    body: Ruma<get_state_events_for_empty_key::Request>,
) -> ConduitResult<get_state_events_for_empty_key::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");

    if !db.rooms.is_joined(sender_id, &body.room_id)? {
        return Err(Error::BadRequest(
            ErrorKind::Forbidden,
            "You don't have permission to view the room state.",
        ));
    }

    let event = db
        .rooms
        .room_state_get(&body.room_id, &body.event_type, "")?
        .ok_or(Error::BadRequest(
            ErrorKind::NotFound,
            "State event not found.",
        ))?;

    Ok(get_state_events_for_empty_key::Response {
        content: serde_json::value::to_raw_value(&event)
            .map_err(|_| Error::bad_database("Invalid event content in database"))?,
    }
    .into())
}

#[cfg_attr(
    feature = "conduit_bin",
    get("/_matrix/client/r0/sync", data = "<body>")
)]
pub async fn sync_events_route(
    db: State<'_, Database>,
    body: Ruma<sync_events::Request>,
) -> ConduitResult<sync_events::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");
    let device_id = body.device_id.as_ref().expect("user is authenticated");

    // TODO: match body.set_presence {
    db.rooms.edus.ping_presence(&sender_id)?;

    // Setup watchers, so if there's no response, we can wait for them
    let watcher = db.watch(sender_id, device_id);

    let next_batch = db.globals.current_count()?.to_string();

    let mut joined_rooms = BTreeMap::new();
    let since = body
        .since
        .clone()
        .and_then(|string| string.parse().ok())
        .unwrap_or(0);

    let mut presence_updates = HashMap::new();
    let mut device_list_updates = HashSet::new();

    for room_id in db.rooms.rooms_joined(&sender_id) {
        let room_id = room_id?;

        let mut non_timeline_pdus = db
            .rooms
            .pdus_since(&sender_id, &room_id, since)?
            .filter_map(|r| r.ok()); // Filter out buggy events

        // Take the last 10 events for the timeline
        let timeline_pdus = non_timeline_pdus
            .by_ref()
            .rev()
            .take(10)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>();

        // They /sync response doesn't always return all messages, so we say the output is
        // limited unless there are events in non_timeline_pdus
        //let mut limited = false;

        let mut state_pdus = Vec::new();
        for pdu in non_timeline_pdus {
            if pdu.state_key.is_some() {
                state_pdus.push(pdu);
            }
        }

        let mut send_member_count = false;
        let mut joined_since_last_sync = false;
        let mut send_notification_counts = false;
        for pdu in db
            .rooms
            .pdus_since(&sender_id, &room_id, since)?
            .filter_map(|r| r.ok())
        {
            send_notification_counts = true;
            if pdu.kind == EventType::RoomMember {
                send_member_count = true;
                if !joined_since_last_sync && pdu.state_key == Some(sender_id.to_string()) {
                    let content = serde_json::from_value::<
                        Raw<ruma::events::room::member::MemberEventContent>,
                    >(pdu.content.clone())
                    .expect("Raw::from_value always works")
                    .deserialize()
                    .map_err(|_| Error::bad_database("Invalid PDU in database."))?;
                    if content.membership == ruma::events::room::member::MembershipState::Join {
                        joined_since_last_sync = true;
                        // Both send_member_count and joined_since_last_sync are set. There's
                        // nothing more to do
                        break;
                    }
                }
            }
        }

        let members = db.rooms.room_state_type(&room_id, &EventType::RoomMember)?;

        let (joined_member_count, invited_member_count, heroes) = if send_member_count {
            let joined_member_count = db.rooms.room_members(&room_id).count();
            let invited_member_count = db.rooms.room_members_invited(&room_id).count();

            // Recalculate heroes (first 5 members)
            let mut heroes = Vec::new();

            if joined_member_count + invited_member_count <= 5 {
                // Go through all PDUs and for each member event, check if the user is still joined or
                // invited until we have 5 or we reach the end

                for hero in db
                    .rooms
                    .all_pdus(&sender_id, &room_id)?
                    .filter_map(|pdu| pdu.ok()) // Ignore all broken pdus
                    .filter(|pdu| pdu.kind == EventType::RoomMember)
                    .map(|pdu| {
                        let content = serde_json::from_value::<
                            Raw<ruma::events::room::member::MemberEventContent>,
                        >(pdu.content.clone())
                        .expect("Raw::from_value always works")
                        .deserialize()
                        .map_err(|_| Error::bad_database("Invalid member event in database."))?;

                        if let Some(state_key) = &pdu.state_key {
                            let current_content = serde_json::from_value::<
                                Raw<ruma::events::room::member::MemberEventContent>,
                            >(
                                members
                                    .get(state_key)
                                    .ok_or_else(|| {
                                        Error::bad_database(
                                            "A user that joined once has no member event anymore.",
                                        )
                                    })?
                                    .content
                                    .clone(),
                            )
                            .expect("Raw::from_value always works")
                            .deserialize()
                            .map_err(|_| {
                                Error::bad_database("Invalid member event in database.")
                            })?;

                            // The membership was and still is invite or join
                            if matches!(
                                content.membership,
                                ruma::events::room::member::MembershipState::Join
                                    | ruma::events::room::member::MembershipState::Invite
                            ) && matches!(
                                current_content.membership,
                                ruma::events::room::member::MembershipState::Join
                                    | ruma::events::room::member::MembershipState::Invite
                            ) {
                                Ok::<_, Error>(Some(state_key.clone()))
                            } else {
                                Ok(None)
                            }
                        } else {
                            Ok(None)
                        }
                    })
                    .filter_map(|u| u.ok()) // Filter out buggy users
                    // Filter for possible heroes
                    .filter_map(|u| u)
                {
                    if heroes.contains(&hero) || hero == sender_id.to_string() {
                        continue;
                    }

                    heroes.push(hero);
                }
            }

            (
                Some(joined_member_count),
                Some(invited_member_count),
                heroes,
            )
        } else {
            (None, None, Vec::new())
        };

        let notification_count = if send_notification_counts {
            if let Some(last_read) = db.rooms.edus.room_read_get(&room_id, &sender_id)? {
                Some(
                    (db.rooms
                        .pdus_since(&sender_id, &room_id, last_read)?
                        .filter_map(|pdu| pdu.ok()) // Filter out buggy events
                        .filter(|pdu| {
                            matches!(
                                pdu.kind.clone(),
                                EventType::RoomMessage | EventType::RoomEncrypted
                            )
                        })
                        .count() as u32)
                        .into(),
                )
            } else {
                None
            }
        } else {
            None
        };

        let prev_batch = timeline_pdus.first().map_or(Ok::<_, Error>(None), |e| {
            Ok(Some(
                db.rooms
                    .get_pdu_count(&e.event_id)?
                    .ok_or_else(|| Error::bad_database("Can't find count from event in db."))?
                    .to_string(),
            ))
        })?;

        let room_events = timeline_pdus
            .into_iter()
            .map(|pdu| pdu.to_sync_room_event())
            .collect::<Vec<_>>();

        let mut edus = db
            .rooms
            .edus
            .roomlatests_since(&room_id, since)?
            .filter_map(|r| r.ok()) // Filter out buggy events
            .collect::<Vec<_>>();

        if db
            .rooms
            .edus
            .last_roomactive_update(&room_id, &db.globals)?
            > since
        {
            edus.push(
                serde_json::from_str(
                    &serde_json::to_string(&AnySyncEphemeralRoomEvent::Typing(
                        db.rooms.edus.roomactives_all(&room_id)?,
                    ))
                    .expect("event is valid, we just created it"),
                )
                .expect("event is valid, we just created it"),
            );
        }

        let joined_room = sync_events::JoinedRoom {
            account_data: sync_events::AccountData {
                events: db
                    .account_data
                    .changes_since(Some(&room_id), &sender_id, since)?
                    .into_iter()
                    .filter_map(|(_, v)| {
                        serde_json::from_str(v.json().get())
                            .map_err(|_| Error::bad_database("Invalid account event in database."))
                            .ok()
                    })
                    .collect::<Vec<_>>(),
            },
            summary: sync_events::RoomSummary {
                heroes,
                joined_member_count: joined_member_count.map(|n| (n as u32).into()),
                invited_member_count: invited_member_count.map(|n| (n as u32).into()),
            },
            unread_notifications: sync_events::UnreadNotificationsCount {
                highlight_count: None,
                notification_count,
            },
            timeline: sync_events::Timeline {
                limited: false || joined_since_last_sync,
                prev_batch,
                events: room_events,
            },
            // TODO: state before timeline
            state: sync_events::State {
                events: if joined_since_last_sync {
                    db.rooms
                        .room_state_full(&room_id)?
                        .into_iter()
                        .map(|(_, pdu)| pdu.to_sync_state_event())
                        .collect()
                } else {
                    Vec::new()
                },
            },
            ephemeral: sync_events::Ephemeral { events: edus },
        };

        if !joined_room.is_empty() {
            joined_rooms.insert(room_id.clone(), joined_room);
        }

        // Look for device list updates in this room
        device_list_updates.extend(
            db.users
                .keys_changed(&room_id, since, None)
                .filter_map(|r| r.ok()),
        );

        // Take presence updates from this room
        for (user_id, presence) in
            db.rooms
                .edus
                .presence_since(&room_id, since, &db.rooms, &db.globals)?
        {
            match presence_updates.entry(user_id) {
                hash_map::Entry::Vacant(v) => {
                    v.insert(presence);
                }
                hash_map::Entry::Occupied(mut o) => {
                    let p = o.get_mut();

                    // Update existing presence event with more info
                    p.content.presence = presence.content.presence;
                    if let Some(status_msg) = presence.content.status_msg {
                        p.content.status_msg = Some(status_msg);
                    }
                    if let Some(last_active_ago) = presence.content.last_active_ago {
                        p.content.last_active_ago = Some(last_active_ago);
                    }
                    if let Some(displayname) = presence.content.displayname {
                        p.content.displayname = Some(displayname);
                    }
                    if let Some(avatar_url) = presence.content.avatar_url {
                        p.content.avatar_url = Some(avatar_url);
                    }
                    if let Some(currently_active) = presence.content.currently_active {
                        p.content.currently_active = Some(currently_active);
                    }
                }
            }
        }
    }

    let mut left_rooms = BTreeMap::new();
    for room_id in db.rooms.rooms_left(&sender_id) {
        let room_id = room_id?;
        let pdus = db.rooms.pdus_since(&sender_id, &room_id, since)?;
        let room_events = pdus
            .filter_map(|pdu| pdu.ok()) // Filter out buggy events
            .map(|pdu| pdu.to_sync_room_event())
            .collect();

        // TODO: Only until leave point
        let mut edus = db
            .rooms
            .edus
            .roomlatests_since(&room_id, since)?
            .filter_map(|r| r.ok()) // Filter out buggy events
            .collect::<Vec<_>>();

        if db
            .rooms
            .edus
            .last_roomactive_update(&room_id, &db.globals)?
            > since
        {
            edus.push(
                serde_json::from_str(
                    &serde_json::to_string(&AnySyncEphemeralRoomEvent::Typing(
                        db.rooms.edus.roomactives_all(&room_id)?,
                    ))
                    .expect("event is valid, we just created it"),
                )
                .expect("event is valid, we just created it"),
            );
        }

        let left_room = sync_events::LeftRoom {
            account_data: sync_events::AccountData { events: Vec::new() },
            timeline: sync_events::Timeline {
                limited: false,
                prev_batch: Some(next_batch.clone()),
                events: room_events,
            },
            state: sync_events::State { events: Vec::new() },
        };

        if !left_room.is_empty() {
            left_rooms.insert(room_id.clone(), left_room);
        }
    }

    let mut invited_rooms = BTreeMap::new();
    for room_id in db.rooms.rooms_invited(&sender_id) {
        let room_id = room_id?;

        let invited_room = sync_events::InvitedRoom {
            invite_state: sync_events::InviteState {
                events: db
                    .rooms
                    .room_state_full(&room_id)?
                    .into_iter()
                    .map(|(_, pdu)| pdu.to_stripped_state_event())
                    .collect(),
            },
        };

        if !invited_room.is_empty() {
            invited_rooms.insert(room_id.clone(), invited_room);
        }
    }

    // Remove all to-device events the device received *last time*
    db.users
        .remove_to_device_events(sender_id, device_id, since)?;

    let response = sync_events::Response {
        next_batch,
        rooms: sync_events::Rooms {
            leave: left_rooms,
            join: joined_rooms,
            invite: invited_rooms,
        },
        presence: sync_events::Presence {
            events: presence_updates
                .into_iter()
                .map(|(_, v)| Raw::from(v))
                .collect(),
        },
        account_data: sync_events::AccountData {
            events: db
                .account_data
                .changes_since(None, &sender_id, since)?
                .into_iter()
                .filter_map(|(_, v)| {
                    serde_json::from_str(v.json().get())
                        .map_err(|_| Error::bad_database("Invalid account event in database."))
                        .ok()
                })
                .collect::<Vec<_>>(),
        },
        device_lists: sync_events::DeviceLists {
            changed: device_list_updates.into_iter().collect(),
            left: Vec::new(), // TODO
        },
        device_one_time_keys_count: Default::default(), // TODO
        to_device: sync_events::ToDevice {
            events: db.users.get_to_device_events(sender_id, device_id)?,
        },
    };

    // TODO: Retry the endpoint instead of returning (waiting for #118)
    if !body.full_state
        && response.rooms.is_empty()
        && response.presence.is_empty()
        && response.account_data.is_empty()
        && response.device_lists.is_empty()
        && response.device_one_time_keys_count.is_empty()
        && response.to_device.is_empty()
    {
        // Hang a few seconds so requests are not spammed
        // Stop hanging if new info arrives
        let mut duration = body.timeout.unwrap_or(Duration::default());
        if duration.as_secs() > 30 {
            duration = Duration::from_secs(30);
        }
        let mut delay = tokio::time::delay_for(duration);
        tokio::select! {
            _ = &mut delay => {}
            _ = watcher => {}
        }
    }

    Ok(response.into())
}

#[cfg_attr(
    feature = "conduit_bin",
    get("/_matrix/client/r0/rooms/<_>/context/<_>", data = "<body>")
)]
pub fn get_context_route(
    db: State<'_, Database>,
    body: Ruma<get_context::Request>,
) -> ConduitResult<get_context::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");

    if !db.rooms.is_joined(sender_id, &body.room_id)? {
        return Err(Error::BadRequest(
            ErrorKind::Forbidden,
            "You don't have permission to view this room.",
        ));
    }

    let base_event = db
        .rooms
        .get_pdu(&body.event_id)?
        .ok_or(Error::BadRequest(
            ErrorKind::NotFound,
            "Base event not found.",
        ))?
        .to_room_event();

    let base_token = db
        .rooms
        .get_pdu_count(&body.event_id)?
        .expect("event still exists");

    let events_before = db
        .rooms
        .pdus_until(&sender_id, &body.room_id, base_token)
        .take(
            u32::try_from(body.limit).map_err(|_| {
                Error::BadRequest(ErrorKind::InvalidParam, "Limit value is invalid.")
            })? as usize
                / 2,
        )
        .filter_map(|r| r.ok()) // Remove buggy events
        .collect::<Vec<_>>();

    let start_token = events_before.last().map(|(count, _)| count.to_string());

    let events_before = events_before
        .into_iter()
        .map(|(_, pdu)| pdu.to_room_event())
        .collect::<Vec<_>>();

    let events_after = db
        .rooms
        .pdus_after(&sender_id, &body.room_id, base_token)
        .take(
            u32::try_from(body.limit).map_err(|_| {
                Error::BadRequest(ErrorKind::InvalidParam, "Limit value is invalid.")
            })? as usize
                / 2,
        )
        .filter_map(|r| r.ok()) // Remove buggy events
        .collect::<Vec<_>>();

    let end_token = events_after.last().map(|(count, _)| count.to_string());

    let events_after = events_after
        .into_iter()
        .map(|(_, pdu)| pdu.to_room_event())
        .collect::<Vec<_>>();

    Ok(get_context::Response {
        start: start_token,
        end: end_token,
        events_before,
        event: Some(base_event),
        events_after,
        state: db // TODO: State at event
            .rooms
            .room_state_full(&body.room_id)?
            .values()
            .map(|pdu| pdu.to_state_event())
            .collect(),
    }
    .into())
}

#[cfg_attr(
    feature = "conduit_bin",
    get("/_matrix/client/r0/rooms/<_>/messages", data = "<body>")
)]
pub fn get_message_events_route(
    db: State<'_, Database>,
    body: Ruma<get_message_events::Request>,
) -> ConduitResult<get_message_events::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");

    if !db.rooms.is_joined(sender_id, &body.room_id)? {
        return Err(Error::BadRequest(
            ErrorKind::Forbidden,
            "You don't have permission to view this room.",
        ));
    }

    let from = body
        .from
        .clone()
        .parse()
        .map_err(|_| Error::BadRequest(ErrorKind::InvalidParam, "Invalid `from` value."))?;

    let to = body.to.as_ref().map(|t| t.parse());

    // Use limit or else 10
    let limit = body
        .limit
        .try_into()
        .map_or(Ok::<_, Error>(10_usize), |l: u32| Ok(l as usize))?;

    match body.dir {
        get_message_events::Direction::Forward => {
            let events_after = db
                .rooms
                .pdus_after(&sender_id, &body.room_id, from)
                .take(limit)
                .filter_map(|r| r.ok()) // Filter out buggy events
                .take_while(|&(k, _)| Some(Ok(k)) != to) // Stop at `to`
                .collect::<Vec<_>>();

            let end_token = events_after.last().map(|(count, _)| count.to_string());

            let events_after = events_after
                .into_iter()
                .map(|(_, pdu)| pdu.to_room_event())
                .collect::<Vec<_>>();

            Ok(get_message_events::Response {
                start: Some(body.from.clone()),
                end: end_token,
                chunk: events_after,
                state: Vec::new(),
            }
            .into())
        }
        get_message_events::Direction::Backward => {
            let events_before = db
                .rooms
                .pdus_until(&sender_id, &body.room_id, from)
                .take(limit)
                .filter_map(|r| r.ok()) // Filter out buggy events
                .take_while(|&(k, _)| Some(Ok(k)) != to) // Stop at `to`
                .collect::<Vec<_>>();

            let start_token = events_before.last().map(|(count, _)| count.to_string());

            let events_before = events_before
                .into_iter()
                .map(|(_, pdu)| pdu.to_room_event())
                .collect::<Vec<_>>();

            Ok(get_message_events::Response {
                start: Some(body.from.clone()),
                end: start_token,
                chunk: events_before,
                state: Vec::new(),
            }
            .into())
        }
    }
}

#[cfg_attr(feature = "conduit_bin", get("/_matrix/client/r0/voip/turnServer"))]
pub fn turn_server_route() -> ConduitResult<create_message_event::Response> {
    Err(Error::BadRequest(
        ErrorKind::NotFound,
        "There is no turn server yet.",
    ))
}

#[cfg_attr(feature = "conduit_bin", post("/_matrix/client/r0/publicised_groups"))]
pub fn publicised_groups_route() -> ConduitResult<create_message_event::Response> {
    Err(Error::BadRequest(
        ErrorKind::NotFound,
        "There are not publicised groups yet.",
    ))
}

#[cfg_attr(
    feature = "conduit_bin",
    put("/_matrix/client/r0/sendToDevice/<_>/<_>", data = "<body>")
)]
pub fn send_event_to_device_route(
    db: State<'_, Database>,
    body: Ruma<send_event_to_device::Request>,
) -> ConduitResult<send_event_to_device::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");

    for (target_user_id, map) in &body.messages {
        for (target_device_id_maybe, event) in map {
            match target_device_id_maybe {
                to_device::DeviceIdOrAllDevices::DeviceId(target_device_id) => {
                    db.users.add_to_device_event(
                        sender_id,
                        &target_user_id,
                        &target_device_id,
                        &body.event_type,
                        serde_json::from_str(event.get()).map_err(|_| {
                            Error::BadRequest(ErrorKind::InvalidParam, "Event is invalid")
                        })?,
                        &db.globals,
                    )?
                }

                to_device::DeviceIdOrAllDevices::AllDevices => {
                    for target_device_id in db.users.all_device_ids(&target_user_id) {
                        db.users.add_to_device_event(
                            sender_id,
                            &target_user_id,
                            &target_device_id?,
                            &body.event_type,
                            serde_json::from_str(event.get()).map_err(|_| {
                                Error::BadRequest(ErrorKind::InvalidParam, "Event is invalid")
                            })?,
                            &db.globals,
                        )?;
                    }
                }
            }
        }
    }

    Ok(send_event_to_device::Response.into())
}

#[cfg_attr(feature = "conduit_bin", get("/_matrix/media/r0/config"))]
pub fn get_media_config_route(
    db: State<'_, Database>,
) -> ConduitResult<get_media_config::Response> {
    Ok(get_media_config::Response {
        upload_size: db.globals.max_request_size().into(),
    }
    .into())
}

#[cfg_attr(
    feature = "conduit_bin",
    post("/_matrix/media/r0/upload", data = "<body>")
)]
pub fn create_content_route(
    db: State<'_, Database>,
    body: Ruma<create_content::Request>,
) -> ConduitResult<create_content::Response> {
    let mxc = format!(
        "mxc://{}/{}",
        db.globals.server_name(),
        utils::random_string(MXC_LENGTH)
    );
    db.media.create(
        mxc.clone(),
        body.filename.as_ref(),
        &body.content_type,
        &body.file,
    )?;

    Ok(create_content::Response { content_uri: mxc }.into())
}

#[cfg_attr(
    feature = "conduit_bin",
    get(
        "/_matrix/media/r0/download/<_server_name>/<_media_id>",
        data = "<body>"
    )
)]
pub fn get_content_route(
    db: State<'_, Database>,
    body: Ruma<get_content::Request>,
    _server_name: String,
    _media_id: String,
) -> ConduitResult<get_content::Response> {
    if let Some((filename, content_type, file)) = db
        .media
        .get(format!("mxc://{}/{}", body.server_name, body.media_id))?
    {
        Ok(get_content::Response {
            file,
            content_type,
            content_disposition: filename.unwrap_or_default(), // TODO: Spec says this should be optional
        }
        .into())
    } else {
        Err(Error::BadRequest(ErrorKind::NotFound, "Media not found."))
    }
}

#[cfg_attr(
    feature = "conduit_bin",
    get(
        "/_matrix/media/r0/thumbnail/<_server_name>/<_media_id>",
        data = "<body>"
    )
)]
pub fn get_content_thumbnail_route(
    db: State<'_, Database>,
    body: Ruma<get_content_thumbnail::Request>,
    _server_name: String,
    _media_id: String,
) -> ConduitResult<get_content_thumbnail::Response> {
    if let Some((_, content_type, file)) = db.media.get_thumbnail(
        format!("mxc://{}/{}", body.server_name, body.media_id),
        body.width
            .try_into()
            .map_err(|_| Error::BadRequest(ErrorKind::InvalidParam, "Width is invalid."))?,
        body.height
            .try_into()
            .map_err(|_| Error::BadRequest(ErrorKind::InvalidParam, "Width is invalid."))?,
    )? {
        Ok(get_content_thumbnail::Response { file, content_type }.into())
    } else {
        Err(Error::BadRequest(ErrorKind::NotFound, "Media not found."))
    }
}

#[cfg_attr(
    feature = "conduit_bin",
    get("/_matrix/client/r0/devices", data = "<body>")
)]
pub fn get_devices_route(
    db: State<'_, Database>,
    body: Ruma<get_devices::Request>,
) -> ConduitResult<get_devices::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");

    let devices = db
        .users
        .all_devices_metadata(sender_id)
        .filter_map(|r| r.ok()) // Filter out buggy devices
        .collect::<Vec<device::Device>>();

    Ok(get_devices::Response { devices }.into())
}

#[cfg_attr(
    feature = "conduit_bin",
    get("/_matrix/client/r0/devices/<_device_id>", data = "<body>")
)]
pub fn get_device_route(
    db: State<'_, Database>,
    body: Ruma<get_device::Request>,
    _device_id: String,
) -> ConduitResult<get_device::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");

    let device = db
        .users
        .get_device_metadata(&sender_id, &body.body.device_id)?
        .ok_or(Error::BadRequest(ErrorKind::NotFound, "Device not found."))?;

    Ok(get_device::Response { device }.into())
}

#[cfg_attr(
    feature = "conduit_bin",
    put("/_matrix/client/r0/devices/<_device_id>", data = "<body>")
)]
pub fn update_device_route(
    db: State<'_, Database>,
    body: Ruma<update_device::Request>,
    _device_id: String,
) -> ConduitResult<update_device::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");

    let mut device = db
        .users
        .get_device_metadata(&sender_id, &body.body.device_id)?
        .ok_or(Error::BadRequest(ErrorKind::NotFound, "Device not found."))?;

    device.display_name = body.display_name.clone();

    db.users
        .update_device_metadata(&sender_id, &body.body.device_id, &device)?;

    Ok(update_device::Response.into())
}

#[cfg_attr(
    feature = "conduit_bin",
    delete("/_matrix/client/r0/devices/<_device_id>", data = "<body>")
)]
pub fn delete_device_route(
    db: State<'_, Database>,
    body: Ruma<delete_device::Request>,
    _device_id: String,
) -> ConduitResult<delete_device::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");
    let device_id = body.device_id.as_ref().expect("user is authenticated");

    // UIAA
    let mut uiaainfo = UiaaInfo {
        flows: vec![AuthFlow {
            stages: vec!["m.login.password".to_owned()],
        }],
        completed: Vec::new(),
        params: Default::default(),
        session: None,
        auth_error: None,
    };

    if let Some(auth) = &body.auth {
        let (worked, uiaainfo) = db.uiaa.try_auth(
            &sender_id,
            &device_id,
            auth,
            &uiaainfo,
            &db.users,
            &db.globals,
        )?;
        if !worked {
            return Err(Error::Uiaa(uiaainfo));
        }
    // Success!
    } else {
        uiaainfo.session = Some(utils::random_string(SESSION_ID_LENGTH));
        db.uiaa.create(&sender_id, &device_id, &uiaainfo)?;
        return Err(Error::Uiaa(uiaainfo));
    }

    db.users.remove_device(&sender_id, &body.body.device_id)?;

    Ok(delete_device::Response.into())
}

#[cfg_attr(
    feature = "conduit_bin",
    post("/_matrix/client/r0/delete_devices", data = "<body>")
)]
pub fn delete_devices_route(
    db: State<'_, Database>,
    body: Ruma<delete_devices::Request>,
) -> ConduitResult<delete_devices::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");
    let device_id = body.device_id.as_ref().expect("user is authenticated");

    // UIAA
    let mut uiaainfo = UiaaInfo {
        flows: vec![AuthFlow {
            stages: vec!["m.login.password".to_owned()],
        }],
        completed: Vec::new(),
        params: Default::default(),
        session: None,
        auth_error: None,
    };

    if let Some(auth) = &body.auth {
        let (worked, uiaainfo) = db.uiaa.try_auth(
            &sender_id,
            &device_id,
            auth,
            &uiaainfo,
            &db.users,
            &db.globals,
        )?;
        if !worked {
            return Err(Error::Uiaa(uiaainfo));
        }
    // Success!
    } else {
        uiaainfo.session = Some(utils::random_string(SESSION_ID_LENGTH));
        db.uiaa.create(&sender_id, &device_id, &uiaainfo)?;
        return Err(Error::Uiaa(uiaainfo));
    }

    for device_id in &body.devices {
        db.users.remove_device(&sender_id, &device_id)?
    }

    Ok(delete_devices::Response.into())
}

#[cfg_attr(
    feature = "conduit_bin",
    post("/_matrix/client/unstable/keys/device_signing/upload", data = "<body>")
)]
pub fn upload_signing_keys_route(
    db: State<'_, Database>,
    body: Ruma<upload_signing_keys::Request>,
) -> ConduitResult<upload_signing_keys::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");
    let device_id = body.device_id.as_ref().expect("user is authenticated");

    // UIAA
    let mut uiaainfo = UiaaInfo {
        flows: vec![AuthFlow {
            stages: vec!["m.login.password".to_owned()],
        }],
        completed: Vec::new(),
        params: Default::default(),
        session: None,
        auth_error: None,
    };

    if let Some(auth) = &body.auth {
        let (worked, uiaainfo) = db.uiaa.try_auth(
            &sender_id,
            &device_id,
            auth,
            &uiaainfo,
            &db.users,
            &db.globals,
        )?;
        if !worked {
            return Err(Error::Uiaa(uiaainfo));
        }
    // Success!
    } else {
        uiaainfo.session = Some(utils::random_string(SESSION_ID_LENGTH));
        db.uiaa.create(&sender_id, &device_id, &uiaainfo)?;
        return Err(Error::Uiaa(uiaainfo));
    }

    if let Some(master_key) = &body.master_key {
        db.users.add_cross_signing_keys(
            sender_id,
            &master_key,
            &body.self_signing_key,
            &body.user_signing_key,
            &db.rooms,
            &db.globals,
        )?;
    }

    Ok(upload_signing_keys::Response.into())
}

#[cfg_attr(
    feature = "conduit_bin",
    post("/_matrix/client/unstable/keys/signatures/upload", data = "<body>")
)]
pub fn upload_signatures_route(
    db: State<'_, Database>,
    body: Ruma<upload_signatures::Request>,
) -> ConduitResult<upload_signatures::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");

    for (user_id, signed_keys) in &body.signed_keys {
        for (key_id, signed_key) in signed_keys {
            for signature in signed_key
                .get("signatures")
                .ok_or(Error::BadRequest(
                    ErrorKind::InvalidParam,
                    "Missing signatures field.",
                ))?
                .get(sender_id.to_string())
                .ok_or(Error::BadRequest(
                    ErrorKind::InvalidParam,
                    "Invalid user in signatures field.",
                ))?
                .as_object()
                .ok_or(Error::BadRequest(
                    ErrorKind::InvalidParam,
                    "Invalid signature.",
                ))?
                .clone()
                .into_iter()
            {
                // Signature validation?
                let signature = (
                    signature.0,
                    signature
                        .1
                        .as_str()
                        .ok_or(Error::BadRequest(
                            ErrorKind::InvalidParam,
                            "Invalid signature value.",
                        ))?
                        .to_owned(),
                );
                db.users.sign_key(
                    &user_id,
                    &key_id,
                    signature,
                    &sender_id,
                    &db.rooms,
                    &db.globals,
                )?;
            }
        }
    }

    Ok(upload_signatures::Response.into())
}

#[cfg_attr(
    feature = "conduit_bin",
    get("/_matrix/client/r0/keys/changes", data = "<body>")
)]
pub fn get_key_changes_route(
    db: State<'_, Database>,
    body: Ruma<get_key_changes::Request>,
) -> ConduitResult<get_key_changes::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");

    let mut device_list_updates = HashSet::new();
    for room_id in db.rooms.rooms_joined(sender_id).filter_map(|r| r.ok()) {
        device_list_updates.extend(
            db.users
                .keys_changed(
                    &room_id,
                    body.from.parse().map_err(|_| {
                        Error::BadRequest(ErrorKind::InvalidParam, "Invalid `from`.")
                    })?,
                    Some(body.to.parse().map_err(|_| {
                        Error::BadRequest(ErrorKind::InvalidParam, "Invalid `to`.")
                    })?),
                )
                .filter_map(|r| r.ok()),
        );
    }
    Ok(get_key_changes::Response {
        changed: device_list_updates.into_iter().collect(),
        left: Vec::new(), // TODO
    }
    .into())
}

#[cfg_attr(feature = "conduit_bin", get("/_matrix/client/r0/pushers"))]
pub fn pushers_route() -> ConduitResult<get_pushers::Response> {
    Ok(get_pushers::Response {
        pushers: Vec::new(),
    }
    .into())
}

#[cfg_attr(feature = "conduit_bin", post("/_matrix/client/r0/pushers/set"))]
pub fn set_pushers_route() -> ConduitResult<get_pushers::Response> {
    Ok(get_pushers::Response {
        pushers: Vec::new(),
    }
    .into())
}

#[cfg_attr(
    feature = "conduit_bin",
    put("/_matrix/client/r0/user/<_>/rooms/<_>/tags/<_>", data = "<body>")
)]
pub fn update_tag_route(
    db: State<'_, Database>,
    body: Ruma<create_tag::Request>,
) -> ConduitResult<create_tag::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");

    let mut tags_event = db
        .account_data
        .get::<ruma::events::tag::TagEvent>(Some(&body.room_id), sender_id, EventType::Tag)?
        .unwrap_or_else(|| ruma::events::tag::TagEvent {
            content: ruma::events::tag::TagEventContent {
                tags: BTreeMap::new(),
            },
        });
    tags_event
        .content
        .tags
        .insert(body.tag.to_string(), body.tag_info.clone());

    db.account_data.update(
        Some(&body.room_id),
        sender_id,
        EventType::Tag,
        &tags_event,
        &db.globals,
    )?;

    Ok(create_tag::Response.into())
}

#[cfg_attr(
    feature = "conduit_bin",
    delete("/_matrix/client/r0/user/<_>/rooms/<_>/tags/<_>", data = "<body>")
)]
pub fn delete_tag_route(
    db: State<'_, Database>,
    body: Ruma<delete_tag::Request>,
) -> ConduitResult<delete_tag::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");

    let mut tags_event = db
        .account_data
        .get::<ruma::events::tag::TagEvent>(Some(&body.room_id), sender_id, EventType::Tag)?
        .unwrap_or_else(|| ruma::events::tag::TagEvent {
            content: ruma::events::tag::TagEventContent {
                tags: BTreeMap::new(),
            },
        });
    tags_event.content.tags.remove(&body.tag);

    db.account_data.update(
        Some(&body.room_id),
        sender_id,
        EventType::Tag,
        &tags_event,
        &db.globals,
    )?;

    Ok(delete_tag::Response.into())
}

#[cfg_attr(
    feature = "conduit_bin",
    get("/_matrix/client/r0/user/<_>/rooms/<_>/tags", data = "<body>")
)]
pub fn get_tags_route(
    db: State<'_, Database>,
    body: Ruma<get_tags::Request>,
) -> ConduitResult<get_tags::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");

    Ok(get_tags::Response {
        tags: db
            .account_data
            .get::<ruma::events::tag::TagEvent>(Some(&body.room_id), sender_id, EventType::Tag)?
            .unwrap_or_else(|| ruma::events::tag::TagEvent {
                content: ruma::events::tag::TagEventContent {
                    tags: BTreeMap::new(),
                },
            })
            .content
            .tags,
    }
    .into())
}

#[cfg(feature = "conduit_bin")]
#[options("/<_..>")]
pub fn options_route() -> ConduitResult<send_event_to_device::Response> {
    Ok(send_event_to_device::Response.into())
}
