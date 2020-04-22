use log::error;
use http::header::{HeaderValue, AUTHORIZATION};
use ruma_api::{
    error::{FromHttpRequestError, FromHttpResponseError},
    Endpoint, Outgoing,
};
use std::convert::{TryFrom, TryInto};

pub async fn send_request<T: Endpoint>(
    data: &crate::Data,
    destination: String,
    request: T,
) -> Option<<T::Response as Outgoing>::Incoming>
where
    // We need to duplicate Endpoint's where clauses because the compiler is not smart enough yet.
    // See https://github.com/rust-lang/rust/issues/54149
    <T as Outgoing>::Incoming: TryFrom<http::Request<Vec<u8>>, Error = FromHttpRequestError>,
    <T::Response as Outgoing>::Incoming: TryFrom<
        http::Response<Vec<u8>>,
        Error = FromHttpResponseError<<T as Endpoint>::ResponseError>,
    >,
    T::Error: std::fmt::Debug,
{
    let mut http_request: http::Request<_> = request.try_into().unwrap();
    let uri = destination.clone() + T::METADATA.path;
    *http_request.uri_mut() = uri.parse().unwrap();

    let body = http_request.body();
    let mut request_json = if !body.is_empty() {
        serde_json::to_value(http_request.body()).unwrap()
    } else {
        serde_json::Map::new().into()
    };

    let request_map = request_json.as_object_mut().unwrap();

    request_map.insert("method".to_owned(), T::METADATA.method.to_string().into());
    request_map.insert("uri".to_owned(), uri.into());
    request_map.insert("origin".to_owned(), data.hostname().into());
    request_map.insert("destination".to_owned(), destination.to_string().into());

    ruma_signatures::sign_json(data.hostname(), data.keypair(), dbg!(&mut request_json)).unwrap();
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
        http_request.headers_mut().insert(AUTHORIZATION, HeaderValue::from_str(dbg!(&format!("X-Matrix origin={},key=\"{}\",sig=\"{}\"", data.hostname(), s.0, s.1))).unwrap());
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
            Some(<T::Response as Outgoing>::Incoming::try_from(dbg!(http_response.body(body).unwrap())).ok().unwrap())
        }
        Err(e) => {
            println!("ERROR: {}", e);
            None
        }
    }
}
