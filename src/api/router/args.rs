use std::{mem, ops::Deref};

use axum::{async_trait, body::Body, extract::FromRequest};
use bytes::{BufMut, BytesMut};
use conduit::{debug, err, trace, utils::string::EMPTY, Error, Result};
use ruma::{api::IncomingRequest, CanonicalJsonValue, OwnedDeviceId, OwnedServerName, OwnedUserId, UserId};
use service::Services;

use super::{auth, auth::Auth, request, request::Request};
use crate::{service::appservice::RegistrationInfo, State};

/// Extractor for Ruma request structs
pub(crate) struct Args<T>
where
	T: IncomingRequest + Send + Sync + 'static,
{
	/// Request struct body
	pub(crate) body: T,

	/// Federation server authentication: X-Matrix origin
	/// None when not a federation server.
	pub(crate) origin: Option<OwnedServerName>,

	/// Local user authentication: user_id.
	/// None when not an authenticated local user.
	pub(crate) sender_user: Option<OwnedUserId>,

	/// Local user authentication: device_id.
	/// None when not an authenticated local user or no device.
	pub(crate) sender_device: Option<OwnedDeviceId>,

	/// Appservice authentication; registration info.
	/// None when not an appservice.
	pub(crate) appservice_info: Option<RegistrationInfo>,

	/// Parsed JSON content.
	/// None when body is not a valid string
	pub(crate) json_body: Option<CanonicalJsonValue>,
}

#[async_trait]
impl<T> FromRequest<State, Body> for Args<T>
where
	T: IncomingRequest + Send + Sync + 'static,
{
	type Rejection = Error;

	async fn from_request(request: hyper::Request<Body>, services: &State) -> Result<Self, Self::Rejection> {
		let mut request = request::from(services, request).await?;
		let mut json_body = serde_json::from_slice::<CanonicalJsonValue>(&request.body).ok();
		let auth = auth::auth(services, &mut request, &json_body, &T::METADATA).await?;
		Ok(Self {
			body: make_body::<T>(services, &mut request, &mut json_body, &auth)?,
			origin: auth.origin,
			sender_user: auth.sender_user,
			sender_device: auth.sender_device,
			appservice_info: auth.appservice_info,
			json_body,
		})
	}
}

impl<T> Deref for Args<T>
where
	T: IncomingRequest + Send + Sync + 'static,
{
	type Target = T;

	fn deref(&self) -> &Self::Target { &self.body }
}

fn make_body<T>(
	services: &Services, request: &mut Request, json_body: &mut Option<CanonicalJsonValue>, auth: &Auth,
) -> Result<T>
where
	T: IncomingRequest + Send + Sync + 'static,
{
	let body = if let Some(CanonicalJsonValue::Object(json_body)) = json_body {
		let user_id = auth.sender_user.clone().unwrap_or_else(|| {
			let server_name = services.globals.server_name();
			UserId::parse_with_server_name(EMPTY, server_name).expect("valid user_id")
		});

		let uiaa_request = json_body
			.get("auth")
			.and_then(CanonicalJsonValue::as_object)
			.and_then(|auth| auth.get("session"))
			.and_then(CanonicalJsonValue::as_str)
			.and_then(|session| {
				services
					.uiaa
					.get_uiaa_request(&user_id, auth.sender_device.as_deref(), session)
			});

		if let Some(CanonicalJsonValue::Object(initial_request)) = uiaa_request {
			for (key, value) in initial_request {
				json_body.entry(key).or_insert(value);
			}
		}

		let mut buf = BytesMut::new().writer();
		serde_json::to_writer(&mut buf, &json_body).expect("value serialization can't fail");
		buf.into_inner().freeze()
	} else {
		mem::take(&mut request.body)
	};

	let mut http_request = hyper::Request::builder()
		.uri(request.parts.uri.clone())
		.method(request.parts.method.clone());
	*http_request.headers_mut().expect("mutable http headers") = request.parts.headers.clone();
	let http_request = http_request.body(body).expect("http request body");

	let headers = http_request.headers();
	let method = http_request.method();
	let uri = http_request.uri();
	debug!("{method:?} {uri:?} {headers:?}");
	trace!("{method:?} {uri:?} {json_body:?}");

	T::try_from_http_request(http_request, &request.path).map_err(|e| err!(Request(BadJson(debug_warn!("{e}")))))
}
