#![feature(proc_macro_hygiene, decl_macro)]
mod ruma_wrapper;

use {
    directories::ProjectDirs,
    rocket::{get, post, put, routes, State},
    ruma_client_api::{
        error::{Error, ErrorKind},
        r0::{
            account::register, alias::get_alias, membership::join_room_by_id,
            message::create_message_event, session::login,
        },
        unversioned::get_supported_versions,
    },
    ruma_identifiers::UserId,
    ruma_wrapper::{MatrixResult, Ruma},
    sled::Db,
    std::{collections::HashMap, convert::TryInto},
};

#[get("/_matrix/client/versions")]
fn get_supported_versions_route() -> MatrixResult<get_supported_versions::Response> {
    MatrixResult(Ok(get_supported_versions::Response {
        versions: vec!["r0.6.0".to_owned()],
        unstable_features: HashMap::new(),
    }))
}

#[post("/_matrix/client/r0/register", data = "<body>")]
fn register_route(
    db: State<Db>,
    body: Ruma<register::Request>,
) -> MatrixResult<register::Response> {
    let users = db.open_tree("users").unwrap();

    let user_id: UserId = match (*format!(
        "@{}:localhost",
        body.username.clone().unwrap_or("randomname".to_owned())
    ))
    .try_into()
    {
        Err(_) => {
            return MatrixResult(Err(Error {
                kind: ErrorKind::InvalidUsername,
                message: "Username was invalid.".to_owned(),
                status_code: http::StatusCode::BAD_REQUEST,
            }))
        }
        Ok(user_id) => user_id,
    };

    if users.contains_key(user_id.to_string()).unwrap() {
        return MatrixResult(Err(Error {
            kind: ErrorKind::UserInUse,
            message: "Desired user ID is already taken.".to_owned(),
            status_code: http::StatusCode::BAD_REQUEST,
        }));
    }

    users
        .insert(
            user_id.to_string(),
            &*body.password.clone().unwrap_or_default(),
        )
        .unwrap();

    MatrixResult(Ok(register::Response {
        access_token: "randomtoken".to_owned(),
        home_server: "localhost".to_owned(),
        user_id,
        device_id: body.device_id.clone().unwrap_or("randomid".to_owned()),
    }))
}

#[post("/_matrix/client/r0/login", data = "<body>")]
fn login_route(db: State<Db>, body: Ruma<login::Request>) -> MatrixResult<login::Response> {
    let user_id = if let login::UserInfo::MatrixId(username) = &body.user {
        let user_id = format!("@{}:localhost", username);
        let users = db.open_tree("users").unwrap();
        if !users.contains_key(user_id.clone()).unwrap() {
            dbg!();
            return MatrixResult(Err(Error {
                kind: ErrorKind::Forbidden,
                message: "UserId not found.".to_owned(),
                status_code: http::StatusCode::BAD_REQUEST,
            }));
        }
        user_id
    } else {
        dbg!();
        return MatrixResult(Err(Error {
            kind: ErrorKind::Unknown,
            message: "Bad login type.".to_owned(),
            status_code: http::StatusCode::BAD_REQUEST,
        }));
    };

    return MatrixResult(Ok(login::Response {
        user_id: (*user_id).try_into().unwrap(), // User id is correct because the user is already registered
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
    // Log info by default
    if let Err(_) = std::env::var("RUST_LOG") {
        std::env::set_var("RUST_LOG", "info");
    }
    pretty_env_logger::init();

    let db = sled::open(
        ProjectDirs::from("xyz", "koesters", "matrixserver")
            .unwrap()
            .data_dir(),
    )
    .unwrap();

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
        .manage(db)
        .launch();
}
