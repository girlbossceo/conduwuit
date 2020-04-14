use super::*;
use rocket::{local::Client, http::Status};
use serde_json::Value;
use serde_json::json;
use ruma_client_api::error::ErrorKind;
use std::time::Duration;

fn setup_client() -> Client {
    Database::try_remove("temp");
    let data = Data::load_or_create("temp");

    let rocket = setup_rocket(data);
    Client::new(rocket).expect("valid rocket instance")
}

#[tokio::test]
async fn register_login() {
    let client = setup_client();
    let mut response = client
        .post("/_matrix/client/r0/register?kind=user")
        .body(registration_init())
        .dispatch().await;
    let body = serde_json::from_str::<Value>(&response.body_string().await.unwrap()).unwrap();

    assert_eq!(response.status().code, 401);
    assert!(dbg!(&body["flows"]).as_array().unwrap().len() > 0);
    assert!(body["session"].as_str().unwrap().len() > 0);
}

#[tokio::test]
async fn login_after_register_correct_password() {
    let client = setup_client();
    let mut response = client
        .post("/_matrix/client/r0/register?kind=user")
        .body(registration_init())
        .dispatch().await;
    let body = serde_json::from_str::<Value>(&response.body_string().await.unwrap()).unwrap();
    let session = body["session"].clone();

    let response = client
        .post("/_matrix/client/r0/register?kind=user")
        .body(registration(session.as_str().unwrap()))
        .dispatch().await;
    assert_eq!(response.status().code, 200);

    let login_response = client
        .post("/_matrix/client/r0/login")
        .body(login_with_password("ilovebananas"))
        .dispatch()
        .await;
    assert_eq!(login_response.status().code, 200);
}

#[tokio::test]
async fn login_after_register_incorrect_password() {
    let client = setup_client();
    let mut response = client
        .post("/_matrix/client/r0/register?kind=user")
        .body(registration_init())
        .dispatch().await;
    let body = serde_json::from_str::<Value>(&response.body_string().await.unwrap()).unwrap();
    let session = body["session"].clone();

    let response = client
        .post("/_matrix/client/r0/register?kind=user")
        .body(registration(session.as_str().unwrap()))
        .dispatch().await;
    assert_eq!(response.status().code, 200);

    let mut login_response = client
        .post("/_matrix/client/r0/login")
        .body(login_with_password("idontlovebananas"))
        .dispatch()
        .await;
    let body = serde_json::from_str::<Value>(&login_response.body_string().await.unwrap()).unwrap();
    assert_eq!(body.as_object().unwrap().get("errcode").unwrap().as_str().unwrap(), "M_FORBIDDEN");
    assert_eq!(login_response.status().code, 403);
}

fn registration_init() -> &'static str {
    r#"{
    "username": "cheeky_monkey",
    "password": "ilovebananas",
    "device_id": "GHTYAJCE",
    "initial_device_display_name": "Jungle Phone",
    "inhibit_login": false
            }"#
}

fn registration(session: &str) -> String {
    json!({
        "auth": {
            "session": session,
            "type": "m.login.dummy"
        },
        "username": "cheeky_monkey",
        "password": "ilovebananas",
        "device_id": "GHTYAJCE",
        "initial_device_display_name": "Jungle Phone",
        "inhibit_login": false
    }).to_string()
}

fn login_with_password(password: &str) -> String {
    json!({
        "type": "m.login.password",
        "identifier": {
            "type": "m.id.user",
            "user": "cheeky_monkey"
        },
        "password": password,
        "initial_device_display_name": "Jungle Phone"
    }).to_string()
}