use crate::{utils, Data, MatrixResult, Ruma};
use http::header::{HeaderValue, AUTHORIZATION};
use log::error;
use rocket::{get, options, post, put, response::content::Json, State};
use ruma_api::{
    error::{FromHttpRequestError, FromHttpResponseError},
    Endpoint,
};
use ruma_client_api::error::{Error, ErrorKind};
use ruma_federation_api::{v1::get_server_version, v2::get_server_keys};
use serde_json::json;
use std::{
    collections::{BTreeMap, HashMap},
    convert::{TryFrom, TryInto},
    path::PathBuf,
    time::{Duration, SystemTime},
};

pub async fn send_request<T: Endpoint>(
    data: &crate::Data,
    destination: String,
    request: T,
) -> Option<T::Response>
{
    let mut http_request: http::Request<_> = request.try_into().unwrap();
    
    *http_request.uri_mut() = format!("https://{}:8448{}", &destination.clone(), T::METADATA.path).parse().unwrap();

    let mut request_map = serde_json::Map::new();

    if !http_request.body().is_empty() {
        request_map.insert("content".to_owned(), 
        serde_json::to_value(http_request.body()).unwrap());
    };

    request_map.insert("method".to_owned(), T::METADATA.method.to_string().into());
    request_map.insert("uri".to_owned(), T::METADATA.path.into());
    request_map.insert("origin".to_owned(), data.hostname().into());
    request_map.insert("destination".to_owned(), destination.to_string().into());
    //request_map.insert("signatures".to_owned(), json!({}));

    let mut request_json = request_map.into();
    ruma_signatures::sign_json(data.hostname(), data.keypair(), dbg!(&mut request_json)).unwrap();
    println!("{}", &request_json);

    let signatures = request_json["signatures"]
        .as_object()
        .unwrap()
        .values()
        .next()
        .unwrap()
        .as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| (k, v.as_str().unwrap()));

    for s in signatures {
        http_request.headers_mut().insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!(
                "X-Matrix origin={},key=\"{}\",sig=\"{}\"",
                data.hostname(),
                s.0,
                s.1
            ))
            .unwrap(),
        );
    }

    let reqwest_response = data
        .reqwest_client()
        .execute(dbg!(http_request.into()))
        .await;

    // Because reqwest::Response -> http::Response is complicated:
    match reqwest_response {
        Ok(mut reqwest_response) => {
            let status = reqwest_response.status();
            let mut http_response = http::Response::builder().status(status);
            let headers = http_response.headers_mut().unwrap();

            for (k, v) in reqwest_response.headers_mut().drain() {
                if let Some(key) = k {
                    headers.insert(key, v);
                }
            }

            let body = reqwest_response
                .bytes()
                .await
                .unwrap()
                .into_iter()
                .collect();
            Some(
                <T::Response>::try_from(
                    dbg!(http_response.body(body)).unwrap(),
                )
                .ok()
                .unwrap(),
            )
        }
        Err(e) => {
            println!("ERROR: {}", e);
            None
        }
    }
}

#[get("/.well-known/matrix/server")]
pub fn well_known_server(data: State<Data>) -> Json<String> {
    rocket::response::content::Json(
        json!({ "m.server": "matrixtesting.koesters.xyz:14004"}).to_string(),
    )
}

#[get("/_matrix/federation/v1/version")]
pub fn get_server_version(data: State<Data>) -> MatrixResult<get_server_version::Response, Error> {
    MatrixResult(Ok(get_server_version::Response {
        server: get_server_version::Server {
            name: Some("Conduit".to_owned()),
            version: Some(env!("CARGO_PKG_VERSION").to_owned()),
        },
    }))
}

#[get("/_matrix/key/v2/server", data = "<body>")]
pub fn get_server_keys(data: State<Data>, body: Ruma<get_server_keys::Request>) -> Json<String> {
    let mut verify_keys = BTreeMap::new();
    verify_keys.insert(
        format!("ed25519:{}", data.keypair().version()),
        get_server_keys::VerifyKey {
            key: base64::encode_config(data.keypair().public_key(), base64::STANDARD_NO_PAD),
        },
    );
    let mut response = serde_json::from_slice(
        http::Response::try_from(get_server_keys::Response {
            server_name: data.hostname().to_owned(),
            verify_keys,
            old_verify_keys: BTreeMap::new(),
            signatures: BTreeMap::new(),
            valid_until_ts: SystemTime::now() + Duration::from_secs(60 * 60 * 24),
        })
        .unwrap()
        .body(),
    )
    .unwrap();
    ruma_signatures::sign_json(data.hostname(), data.keypair(), &mut response).unwrap();
    Json(dbg!(response.to_string()))
}

#[get("/_matrix/key/v2/server/<_key_id>", data = "<body>")]
pub fn get_server_keys_deprecated(
    data: State<Data>,
    body: Ruma<get_server_keys::Request>,
    _key_id: String,
) -> Json<String> {
    get_server_keys(data, body)
}
