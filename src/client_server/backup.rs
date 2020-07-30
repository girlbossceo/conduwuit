use super::State;
use crate::{ConduitResult, Database, Error, Ruma};
use ruma::api::client::{
    error::ErrorKind,
    r0::backup::{
        add_backup_keys, create_backup, get_backup, get_backup_keys, get_latest_backup,
        update_backup,
    },
};

#[cfg(feature = "conduit_bin")]
use rocket::{get, post, put};

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
