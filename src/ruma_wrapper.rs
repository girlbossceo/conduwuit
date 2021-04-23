use crate::Error;
use ruma::{
    api::OutgoingResponse,
    identifiers::{DeviceId, UserId},
    Outgoing,
};
use std::ops::Deref;

#[cfg(feature = "conduit_bin")]
use {
    crate::{server_server, utils},
    log::{debug, warn},
    rocket::{
        data::{self, ByteUnit, Data, FromData},
        http::Status,
        outcome::Outcome::*,
        response::{self, Responder},
        tokio::io::AsyncReadExt,
        Request, State,
    },
    ruma::{
        api::{AuthScheme, IncomingRequest},
        signatures::CanonicalJsonValue,
        ServerName,
    },
    std::collections::BTreeMap,
    std::convert::TryFrom,
    std::io::Cursor,
};

/// This struct converts rocket requests into ruma structs by converting them into http requests
/// first.
pub struct Ruma<T: Outgoing> {
    pub body: T::Incoming,
    pub sender_user: Option<UserId>,
    pub sender_device: Option<Box<DeviceId>>,
    pub json_body: Option<Box<serde_json::value::RawValue>>, // This is None when body is not a valid string
    pub from_appservice: bool,
}

#[cfg(feature = "conduit_bin")]
#[rocket::async_trait]
impl<'a, T: Outgoing> FromData<'a> for Ruma<T>
where
    T::Incoming: IncomingRequest,
{
    type Error = ();

    async fn from_data(request: &'a Request<'_>, data: Data) -> data::Outcome<Self, Self::Error> {
        let metadata = T::Incoming::METADATA;
        let db = request
            .guard::<State<'_, crate::Database>>()
            .await
            .expect("database was loaded");

        // Get token from header or query value
        let token = request
            .headers()
            .get_one("Authorization")
            .map(|s| s[7..].to_owned()) // Split off "Bearer "
            .or_else(|| request.query_value("access_token").and_then(|r| r.ok()));

        let limit = db.globals.max_request_size();
        let mut handle = data.open(ByteUnit::Byte(limit.into()));
        let mut body = Vec::new();
        handle.read_to_end(&mut body).await.unwrap();

        let (sender_user, sender_device, from_appservice) = if let Some((_id, registration)) = db
            .appservice
            .iter_all()
            .filter_map(|r| r.ok())
            .find(|(_id, registration)| {
                registration
                    .get("as_token")
                    .and_then(|as_token| as_token.as_str())
                    .map_or(false, |as_token| token.as_deref() == Some(as_token))
            }) {
            match metadata.authentication {
                AuthScheme::AccessToken | AuthScheme::QueryOnlyAccessToken => {
                    let user_id = request.query_value::<String>("user_id").map_or_else(
                        || {
                            UserId::parse_with_server_name(
                                registration
                                    .get("sender_localpart")
                                    .unwrap()
                                    .as_str()
                                    .unwrap(),
                                db.globals.server_name(),
                            )
                            .unwrap()
                        },
                        |string| {
                            UserId::try_from(string.expect("parsing to string always works"))
                                .unwrap()
                        },
                    );

                    if !db.users.exists(&user_id).unwrap() {
                        // Forbidden
                        return Failure((Status::raw(580), ()));
                    }

                    // TODO: Check if appservice is allowed to be that user
                    (Some(user_id), None, true)
                }
                AuthScheme::ServerSignatures => (None, None, true),
                AuthScheme::None => (None, None, true),
            }
        } else {
            match metadata.authentication {
                AuthScheme::AccessToken | AuthScheme::QueryOnlyAccessToken => {
                    if let Some(token) = token {
                        match db.users.find_from_token(&token).unwrap() {
                            // Unknown Token
                            None => return Failure((Status::raw(581), ())),
                            Some((user_id, device_id)) => {
                                (Some(user_id), Some(device_id.into()), false)
                            }
                        }
                    } else {
                        // Missing Token
                        return Failure((Status::raw(582), ()));
                    }
                }
                AuthScheme::ServerSignatures => {
                    // Get origin from header
                    let x_matrix = match request
                            .headers()
                            .get_one("Authorization")
                            .map(|s| {
                                s[9..]
                                    .split_terminator(',').map(|field| {let mut splits = field.splitn(2, '='); (splits.next(), splits.next().map(|s| s.trim_matches('"')))}).collect::<BTreeMap<_, _>>()
                            }) // Split off "X-Matrix " and parse the rest
                        {
                            Some(t) => t,
                            None => {
                                warn!("No Authorization header");

                                // Forbidden
                                return Failure((Status::raw(580), ()));
                            }
                        };

                    let origin_str = match x_matrix.get(&Some("origin")) {
                        Some(Some(o)) => *o,
                        _ => {
                            warn!("Invalid X-Matrix header origin field: {:?}", x_matrix);

                            // Forbidden
                            return Failure((Status::raw(580), ()));
                        }
                    };

                    let origin = match Box::<ServerName>::try_from(origin_str) {
                        Ok(s) => s,
                        _ => {
                            warn!(
                                "Invalid server name in X-Matrix header origin field: {:?}",
                                x_matrix
                            );

                            // Forbidden
                            return Failure((Status::raw(580), ()));
                        }
                    };

                    let key = match x_matrix.get(&Some("key")) {
                        Some(Some(k)) => *k,
                        _ => {
                            warn!("Invalid X-Matrix header key field: {:?}", x_matrix);

                            // Forbidden
                            return Failure((Status::raw(580), ()));
                        }
                    };

                    let sig = match x_matrix.get(&Some("sig")) {
                        Some(Some(s)) => *s,
                        _ => {
                            warn!("Invalid X-Matrix header sig field: {:?}", x_matrix);

                            // Forbidden
                            return Failure((Status::raw(580), ()));
                        }
                    };

                    let json_body = serde_json::from_slice::<CanonicalJsonValue>(&body);

                    let mut request_map = BTreeMap::<String, CanonicalJsonValue>::new();

                    if let Ok(json_body) = json_body {
                        request_map.insert("content".to_owned(), json_body);
                    };

                    request_map.insert(
                        "method".to_owned(),
                        CanonicalJsonValue::String(request.method().to_string()),
                    );
                    request_map.insert(
                        "uri".to_owned(),
                        CanonicalJsonValue::String(request.uri().to_string()),
                    );

                    println!("{}: {:?}", origin, request.uri().to_string());

                    request_map.insert(
                        "origin".to_owned(),
                        CanonicalJsonValue::String(origin.as_str().to_owned()),
                    );
                    request_map.insert(
                        "destination".to_owned(),
                        CanonicalJsonValue::String(db.globals.server_name().as_str().to_owned()),
                    );

                    let mut origin_signatures = BTreeMap::new();
                    origin_signatures
                        .insert(key.to_owned(), CanonicalJsonValue::String(sig.to_owned()));

                    let mut signatures = BTreeMap::new();
                    signatures.insert(
                        origin.as_str().to_owned(),
                        CanonicalJsonValue::Object(origin_signatures),
                    );

                    request_map.insert(
                        "signatures".to_owned(),
                        CanonicalJsonValue::Object(signatures),
                    );

                    let keys = match server_server::fetch_signing_keys(
                        &db,
                        &origin,
                        vec![&key.to_owned()],
                    )
                    .await
                    {
                        Ok(b) => b,
                        Err(e) => {
                            warn!("Failed to fetch signing keys: {}", e);

                            // Forbidden
                            return Failure((Status::raw(580), ()));
                        }
                    };

                    let mut pub_key_map = BTreeMap::new();
                    pub_key_map.insert(origin.as_str().to_owned(), keys);

                    match ruma::signatures::verify_json(&pub_key_map, &request_map) {
                        Ok(()) => (None, None, false),
                        Err(e) => {
                            warn!("Failed to verify json request from {}: {}", origin, e,);

                            // Forbidden
                            return Failure((Status::raw(580), ()));
                        }
                    }
                }
                AuthScheme::None => (None, None, false),
            }
        };

        let mut http_request = http::Request::builder()
            .uri(request.uri().to_string())
            .method(&*request.method().to_string());
        for header in request.headers().iter() {
            http_request = http_request.header(header.name.as_str(), &*header.value);
        }

        let http_request = http_request.body(&*body).unwrap();
        debug!("{:?}", http_request);
        match <T::Incoming as IncomingRequest>::try_from_http_request(http_request) {
            Ok(t) => Success(Ruma {
                body: t,
                sender_user,
                sender_device,
                // TODO: Can we avoid parsing it again? (We only need this for append_pdu)
                json_body: utils::string_from_bytes(&body)
                    .ok()
                    .and_then(|s| serde_json::value::RawValue::from_string(s).ok()),
                from_appservice,
            }),
            Err(e) => {
                warn!("{:?}", e);
                Failure((Status::raw(583), ()))
            }
        }
    }
}

impl<T: Outgoing> Deref for Ruma<T> {
    type Target = T::Incoming;

    fn deref(&self) -> &Self::Target {
        &self.body
    }
}

/// This struct converts ruma responses into rocket http responses.
pub type ConduitResult<T> = std::result::Result<RumaResponse<T>, Error>;

pub struct RumaResponse<T: OutgoingResponse>(pub T);

impl<T: OutgoingResponse> From<T> for RumaResponse<T> {
    fn from(t: T) -> Self {
        Self(t)
    }
}

#[cfg(feature = "conduit_bin")]
impl<'r, 'o, T> Responder<'r, 'o> for RumaResponse<T>
where
    T: Send + OutgoingResponse,
    'o: 'r,
{
    fn respond_to(self, _: &'r Request<'_>) -> response::Result<'o> {
        let http_response: Result<http::Response<_>, _> = self.0.try_into_http_response();
        match http_response {
            Ok(http_response) => {
                let mut response = rocket::response::Response::build();

                let status = http_response.status();
                response.raw_status(status.into(), "");

                for header in http_response.headers() {
                    response
                        .raw_header(header.0.to_string(), header.1.to_str().unwrap().to_owned());
                }

                let http_body = http_response.into_body();

                response.sized_body(http_body.len(), Cursor::new(http_body));

                response.raw_header("Access-Control-Allow-Origin", "*");
                response.raw_header(
                    "Access-Control-Allow-Methods",
                    "GET, POST, PUT, DELETE, OPTIONS",
                );
                response.raw_header(
                    "Access-Control-Allow-Headers",
                    "Origin, X-Requested-With, Content-Type, Accept, Authorization",
                );
                response.raw_header("Access-Control-Max-Age", "86400");
                response.ok()
            }
            Err(_) => Err(Status::InternalServerError),
        }
    }
}
