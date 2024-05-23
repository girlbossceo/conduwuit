use std::{mem, str};

use axum::{
	async_trait,
	extract::{FromRequest, Path},
	RequestExt, RequestPartsExt,
};
use axum_extra::{
	headers::{authorization::Bearer, Authorization},
	TypedHeader,
};
use bytes::{BufMut, Bytes, BytesMut};
use conduit::debug_warn;
use http::request::Parts;
use ruma::{
	api::{client::error::ErrorKind, IncomingRequest},
	CanonicalJsonValue, UserId,
};
use serde::Deserialize;
use tracing::{debug, trace, warn};

use super::{auth, auth::Auth, Ruma};
use crate::{services, Error, Result};

#[derive(Deserialize)]
pub(super) struct QueryParams {
	pub(super) access_token: Option<String>,
	pub(super) user_id: Option<String>,
}

pub(super) struct Request {
	pub(super) auth: Option<TypedHeader<Authorization<Bearer>>>,
	pub(super) path: Path<Vec<String>>,
	pub(super) query: QueryParams,
	pub(super) json: Option<CanonicalJsonValue>,
	pub(super) body: Bytes,
	pub(super) parts: Parts,
}

#[async_trait]
impl<T, S> FromRequest<S, axum::body::Body> for Ruma<T>
where
	T: IncomingRequest,
{
	type Rejection = Error;

	async fn from_request(request: hyper::Request<axum::body::Body>, _state: &S) -> Result<Self, Self::Rejection> {
		let mut request: Request = extract(request).await?;
		let auth: Auth = auth::auth::<T>(&mut request).await?;
		let body = make_body::<T>(&mut request, &auth)?;
		Ok(Ruma {
			body,
			sender_user: auth.sender_user,
			sender_device: auth.sender_device,
			origin: auth.origin,
			json_body: request.json,
			appservice_info: auth.appservice_info,
		})
	}
}

fn make_body<T>(request: &mut Request, auth: &Auth) -> Result<T>
where
	T: IncomingRequest,
{
	let body = if let Some(CanonicalJsonValue::Object(json_body)) = &mut request.json {
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
		serde_json::to_writer(&mut buf, &request.json).expect("value serialization can't fail");
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

	trace!("{:?} {:?} {:?}", http_request.method(), http_request.uri(), request.json);
	let body = T::try_from_http_request(http_request, &request.path).map_err(|e| {
		warn!("try_from_http_request failed: {e:?}",);
		debug_warn!("JSON body: {:?}", request.json);
		Error::BadRequest(ErrorKind::BadJson, "Failed to deserialize request.")
	})?;

	Ok(body)
}

async fn extract(request: hyper::Request<axum::body::Body>) -> Result<Request> {
	let limited = request.with_limited_body();
	let (mut parts, body) = limited.into_parts();

	let auth = parts.extract().await?;
	let path = parts.extract().await?;
	let query = serde_html_form::from_str(parts.uri.query().unwrap_or_default())
		.map_err(|_| Error::BadRequest(ErrorKind::Unknown, "Failed to read query parameters"))?;

	let max_body_size = services()
		.globals
		.config
		.max_request_size
		.try_into()
		.expect("failed to convert max request size");

	let body = axum::body::to_bytes(body, max_body_size)
		.await
		.map_err(|_| Error::BadRequest(ErrorKind::TooLarge, "Request body too large"))?;

	let json = serde_json::from_slice::<CanonicalJsonValue>(&body).ok();

	Ok(Request {
		auth,
		path,
		query,
		json,
		body,
		parts,
	})
}
