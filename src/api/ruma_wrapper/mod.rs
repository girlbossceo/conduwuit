mod auth;
mod handler;
mod request;
mod xmatrix;

use std::{mem, ops::Deref};

use axum::{async_trait, body::Body, extract::FromRequest};
use bytes::{BufMut, BytesMut};
pub(super) use conduit::error::RumaResponse;
use conduit::{debug, debug_warn, trace, warn};
use ruma::{
	api::{client::error::ErrorKind, IncomingRequest},
	CanonicalJsonValue, OwnedDeviceId, OwnedServerName, OwnedUserId, UserId,
};

pub(super) use self::handler::RouterExt;
use self::{auth::Auth, request::Request};
use crate::{service::appservice::RegistrationInfo, services, Error, Result};

/// Extractor for Ruma request structs
pub(crate) struct Ruma<T> {
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
impl<T, S> FromRequest<S, Body> for Ruma<T>
where
	T: IncomingRequest,
{
	type Rejection = Error;

	async fn from_request(request: hyper::Request<Body>, _: &S) -> Result<Self, Self::Rejection> {
		let mut request = request::from(request).await?;
		let mut json_body = serde_json::from_slice::<CanonicalJsonValue>(&request.body).ok();
		let auth = auth::auth(&mut request, &json_body, &T::METADATA).await?;
		Ok(Self {
			body: make_body::<T>(&mut request, &mut json_body, &auth)?,
			origin: auth.origin,
			sender_user: auth.sender_user,
			sender_device: auth.sender_device,
			appservice_info: auth.appservice_info,
			json_body,
		})
	}
}

impl<T> Deref for Ruma<T> {
	type Target = T;

	fn deref(&self) -> &Self::Target { &self.body }
}

fn make_body<T>(request: &mut Request, json_body: &mut Option<CanonicalJsonValue>, auth: &Auth) -> Result<T>
where
	T: IncomingRequest,
{
	let body = if let Some(CanonicalJsonValue::Object(json_body)) = json_body {
		let user_id = auth.sender_user.clone().unwrap_or_else(|| {
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
					&auth.sender_device.clone().unwrap_or_else(|| "".into()),
					session,
				)
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
	*http_request.headers_mut().unwrap() = request.parts.headers.clone();
	let http_request = http_request.body(body).unwrap();
	debug!(
		"{:?} {:?} {:?}",
		http_request.method(),
		http_request.uri(),
		http_request.headers()
	);

	trace!("{:?} {:?} {:?}", http_request.method(), http_request.uri(), json_body);
	let body = T::try_from_http_request(http_request, &request.path).map_err(|e| {
		warn!("try_from_http_request failed: {e:?}",);
		debug_warn!("JSON body: {:?}", json_body);
		Error::BadRequest(ErrorKind::BadJson, "Failed to deserialize request.")
	})?;

	Ok(body)
}
