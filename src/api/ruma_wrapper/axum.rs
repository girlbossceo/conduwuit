use std::{collections::BTreeMap, str};

use axum::{
	async_trait,
	extract::{FromRequest, Path},
	response::{IntoResponse, Response},
	RequestExt, RequestPartsExt,
};
use axum_extra::{
	headers::{
		authorization::{Bearer, Credentials},
		Authorization,
	},
	typed_header::TypedHeaderRejectionReason,
	TypedHeader,
};
use bytes::{BufMut, BytesMut};
use http::{uri::PathAndQuery, StatusCode};
use http_body_util::Full;
use hyper::Request;
use ruma::{
	api::{client::error::ErrorKind, AuthScheme, IncomingRequest, OutgoingResponse},
	CanonicalJsonValue, OwnedDeviceId, OwnedServerName, OwnedUserId, UserId,
};
use serde::Deserialize;
use tracing::{debug, error, trace, warn};

use super::{Ruma, RumaResponse};
use crate::{debug_warn, service::appservice::RegistrationInfo, services, Error, Result};

enum Token {
	Appservice(Box<RegistrationInfo>),
	User((OwnedUserId, OwnedDeviceId)),
	Invalid,
	None,
}

#[derive(Deserialize)]
struct QueryParams {
	access_token: Option<String>,
	user_id: Option<String>,
}

#[async_trait]
impl<T, S> FromRequest<S, axum::body::Body> for Ruma<T>
where
	T: IncomingRequest,
{
	type Rejection = Error;

	#[allow(unused_qualifications)] // async traits
	async fn from_request(req: Request<axum::body::Body>, _state: &S) -> Result<Self, Self::Rejection> {
		let limited = req.with_limited_body();
		let (mut parts, body) = limited.into_parts();
		let mut body = axum::body::to_bytes(
			body,
			services()
				.globals
				.config
				.max_request_size
				.try_into()
				.expect("failed to convert max request size"),
		)
		.await
		.map_err(|_| Error::BadRequest(ErrorKind::MissingToken, "Missing token."))?;

		let metadata = T::METADATA;
		let auth_header: Option<TypedHeader<Authorization<Bearer>>> = parts.extract().await?;
		let path_params: Path<Vec<String>> = parts.extract().await?;

		let query = parts.uri.query().unwrap_or_default();
		let query_params: QueryParams = match serde_html_form::from_str(query) {
			Ok(params) => params,
			Err(e) => {
				error!(%query, "Failed to deserialize query parameters: {e}");
				return Err(Error::BadRequest(ErrorKind::Unknown, "Failed to read query parameters"));
			},
		};

		let token = match &auth_header {
			Some(TypedHeader(Authorization(bearer))) => Some(bearer.token()),
			None => query_params.access_token.as_deref(),
		};

		let token = if let Some(token) = token {
			if let Some(reg_info) = services().appservice.find_from_token(token).await {
				Token::Appservice(Box::new(reg_info))
			} else if let Some((user_id, device_id)) = services().users.find_from_token(token)? {
				Token::User((user_id, OwnedDeviceId::from(device_id)))
			} else {
				Token::Invalid
			}
		} else {
			Token::None
		};

		if metadata.authentication == AuthScheme::None {
			match parts.uri.path() {
				// TODO: can we check this better?
				"/_matrix/client/v3/publicRooms" | "/_matrix/client/r0/publicRooms" => {
					if !services()
						.globals
						.config
						.allow_public_room_directory_without_auth
					{
						match token {
							Token::Appservice(_) | Token::User(_) => {
								// we should have validated the token above
								// already
							},
							Token::None | Token::Invalid => {
								return Err(Error::BadRequest(
									ErrorKind::MissingToken,
									"Missing or invalid access token.",
								));
							},
						}
					}
				},
				_ => {},
			};
		}

		let mut json_body = serde_json::from_slice::<CanonicalJsonValue>(&body).ok();

		let (sender_user, sender_device, sender_servername, appservice_info) = match (metadata.authentication, token) {
			(_, Token::Invalid) => {
				return Err(Error::BadRequest(
					ErrorKind::UnknownToken {
						soft_logout: false,
					},
					"Unknown access token.",
				))
			},
			(AuthScheme::AccessToken, Token::Appservice(info)) => {
				let user_id = query_params
					.user_id
					.map_or_else(
						|| {
							UserId::parse_with_server_name(
								info.registration.sender_localpart.as_str(),
								services().globals.server_name(),
							)
						},
						UserId::parse,
					)
					.map_err(|_| Error::BadRequest(ErrorKind::InvalidUsername, "Username is invalid."))?;

				if !info.is_user_match(&user_id) {
					return Err(Error::BadRequest(ErrorKind::Exclusive, "User is not in namespace."));
				}

				if !services().users.exists(&user_id)? {
					return Err(Error::BadRequest(ErrorKind::forbidden(), "User does not exist."));
				}

				(Some(user_id), None, None, Some(*info))
			},
			(
				AuthScheme::None | AuthScheme::AccessTokenOptional | AuthScheme::AppserviceToken,
				Token::Appservice(info),
			) => (None, None, None, Some(*info)),
			(AuthScheme::AccessToken, Token::None) => {
				return Err(Error::BadRequest(ErrorKind::MissingToken, "Missing access token."));
			},
			(
				AuthScheme::AccessToken | AuthScheme::AccessTokenOptional | AuthScheme::None,
				Token::User((user_id, device_id)),
			) => (Some(user_id), Some(device_id), None, None),
			(AuthScheme::ServerSignatures, Token::None) => {
				if !services().globals.allow_federation() {
					return Err(Error::bad_config("Federation is disabled."));
				}

				let TypedHeader(Authorization(x_matrix)) = parts
					.extract::<TypedHeader<Authorization<XMatrix>>>()
					.await
					.map_err(|e| {
						warn!("Missing or invalid Authorization header: {e}");

						let msg = match e.reason() {
							TypedHeaderRejectionReason::Missing => "Missing Authorization header.",
							TypedHeaderRejectionReason::Error(_) => "Invalid X-Matrix signatures.",
							_ => "Unknown header-related error",
						};

						Error::BadRequest(ErrorKind::forbidden(), msg)
					})?;

				let origin_signatures =
					BTreeMap::from_iter([(x_matrix.key.clone(), CanonicalJsonValue::String(x_matrix.sig))]);

				let signatures = BTreeMap::from_iter([(
					x_matrix.origin.as_str().to_owned(),
					CanonicalJsonValue::Object(origin_signatures),
				)]);

				let server_destination = services().globals.server_name().as_str().to_owned();

				if let Some(destination) = x_matrix.destination.as_ref() {
					if destination != &server_destination {
						return Err(Error::BadRequest(ErrorKind::forbidden(), "Invalid authorization."));
					}
				}

				let signature_uri = CanonicalJsonValue::String(
					parts
						.uri
						.path_and_query()
						.unwrap_or(&PathAndQuery::from_static("/"))
						.to_string(),
				);

				let mut request_map = BTreeMap::from_iter([
					("method".to_owned(), CanonicalJsonValue::String(parts.method.to_string())),
					("uri".to_owned(), signature_uri),
					(
						"origin".to_owned(),
						CanonicalJsonValue::String(x_matrix.origin.as_str().to_owned()),
					),
					("destination".to_owned(), CanonicalJsonValue::String(server_destination)),
					("signatures".to_owned(), CanonicalJsonValue::Object(signatures)),
				]);

				if let Some(json_body) = &json_body {
					request_map.insert("content".to_owned(), json_body.clone());
				};

				let keys_result = services()
					.rooms
					.event_handler
					.fetch_signing_keys_for_server(&x_matrix.origin, vec![x_matrix.key.clone()])
					.await;

				let keys = keys_result.map_err(|e| {
					warn!("Failed to fetch signing keys: {e}");
					Error::BadRequest(ErrorKind::forbidden(), "Failed to fetch signing keys.")
				})?;

				let pub_key_map = BTreeMap::from_iter([(x_matrix.origin.as_str().to_owned(), keys)]);

				match ruma::signatures::verify_json(&pub_key_map, &request_map) {
					Ok(()) => (None, None, Some(x_matrix.origin), None),
					Err(e) => {
						warn!("Failed to verify json request from {}: {e}\n{request_map:?}", x_matrix.origin);

						if parts.uri.to_string().contains('@') {
							warn!(
								"Request uri contained '@' character. Make sure your reverse proxy gives Conduit the \
								 raw uri (apache: use nocanon)"
							);
						}

						return Err(Error::BadRequest(
							ErrorKind::forbidden(),
							"Failed to verify X-Matrix signatures.",
						));
					},
				}
			},
			(AuthScheme::None | AuthScheme::AppserviceToken | AuthScheme::AccessTokenOptional, Token::None) => {
				(None, None, None, None)
			},
			(AuthScheme::ServerSignatures, Token::Appservice(_) | Token::User(_)) => {
				return Err(Error::BadRequest(
					ErrorKind::Unauthorized,
					"Only server signatures should be used on this endpoint.",
				));
			},
			(AuthScheme::AppserviceToken, Token::User(_)) => {
				return Err(Error::BadRequest(
					ErrorKind::Unauthorized,
					"Only appservice access tokens should be used on this endpoint.",
				));
			},
		};

		let mut http_request = Request::builder().uri(parts.uri).method(parts.method);
		*http_request.headers_mut().unwrap() = parts.headers;

		if let Some(CanonicalJsonValue::Object(json_body)) = &mut json_body {
			let user_id = sender_user.clone().unwrap_or_else(|| {
				UserId::parse_with_server_name("", services().globals.server_name()).expect("we know this is valid")
			});

			let uiaa_request = json_body
				.get("auth")
				.and_then(|auth| auth.as_object())
				.and_then(|auth| auth.get("session"))
				.and_then(|session| session.as_str())
				.and_then(|session| {
					services().uiaa.get_uiaa_request(
						&user_id,
						&sender_device.clone().unwrap_or_else(|| "".into()),
						session,
					)
				});

			if let Some(CanonicalJsonValue::Object(initial_request)) = uiaa_request {
				for (key, value) in initial_request {
					json_body.entry(key).or_insert(value);
				}
			}

			let mut buf = BytesMut::new().writer();
			serde_json::to_writer(&mut buf, json_body).expect("value serialization can't fail");
			body = buf.into_inner().freeze();
		}

		let http_request = http_request.body(&*body).unwrap();
		debug!(
			"{:?} {:?} {:?}",
			http_request.method(),
			http_request.uri(),
			http_request.headers()
		);

		trace!("{:?} {:?} {:?}", http_request.method(), http_request.uri(), json_body);
		let body = T::try_from_http_request(http_request, &path_params).map_err(|e| {
			warn!("try_from_http_request failed: {e:?}",);
			debug_warn!("JSON body: {:?}", json_body);
			Error::BadRequest(ErrorKind::BadJson, "Failed to deserialize request.")
		})?;

		Ok(Ruma {
			body,
			sender_user,
			sender_device,
			sender_servername,
			json_body,
			appservice_info,
		})
	}
}

struct XMatrix {
	origin: OwnedServerName,
	destination: Option<String>,
	key: String, // KeyName?
	sig: String,
}

impl Credentials for XMatrix {
	const SCHEME: &'static str = "X-Matrix";

	fn decode(value: &http::HeaderValue) -> Option<Self> {
		debug_assert!(
			value.as_bytes().starts_with(b"X-Matrix "),
			"HeaderValue to decode should start with \"X-Matrix ..\", received = {value:?}",
		);

		let parameters = str::from_utf8(&value.as_bytes()["X-Matrix ".len()..])
			.ok()?
			.trim_start();

		let mut origin = None;
		let mut destination = None;
		let mut key = None;
		let mut sig = None;

		for entry in parameters.split_terminator(',') {
			let (name, value) = entry.split_once('=')?;

			// It's not at all clear why some fields are quoted and others not in the spec,
			// let's simply accept either form for every field.
			let value = value
				.strip_prefix('"')
				.and_then(|rest| rest.strip_suffix('"'))
				.unwrap_or(value);

			// FIXME: Catch multiple fields of the same name
			match name {
				"origin" => origin = Some(value.try_into().ok()?),
				"key" => key = Some(value.to_owned()),
				"sig" => sig = Some(value.to_owned()),
				"destination" => destination = Some(value.to_owned()),
				_ => debug!("Unexpected field `{name}` in X-Matrix Authorization header"),
			}
		}

		Some(Self {
			origin: origin?,
			key: key?,
			sig: sig?,
			destination,
		})
	}

	fn encode(&self) -> http::HeaderValue { todo!() }
}

impl<T: OutgoingResponse> IntoResponse for RumaResponse<T> {
	fn into_response(self) -> Response {
		match self.0.try_into_http_response::<BytesMut>() {
			Ok(res) => res.map(BytesMut::freeze).map(Full::new).into_response(),
			Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
		}
	}
}
