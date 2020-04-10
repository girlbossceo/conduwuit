use super::*;
use rocket::{local::Client, http::Status};

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
        .body(
            r#"{
    "username": "cheeky_monkey",
    "password": "ilovebananas",
    "device_id": "GHTYAJCE",
    "initial_device_display_name": "Jungle Phone",
    "inhibit_login": false
            }"#,
        )
        .dispatch().await;
    let body = serde_json::to_value(&response.body_string().await.unwrap()).unwrap();

    assert_eq!(response.status().code, 401);
    assert!(dbg!(&body["flows"]).as_array().unwrap().len() > 0);
    assert!(body["session"].as_str().unwrap().len() > 0);
}
