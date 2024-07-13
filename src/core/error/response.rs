use bytes::BytesMut;
use http::StatusCode;
use http_body_util::Full;
use ruma::api::{client::uiaa::UiaaResponse, OutgoingResponse};

use super::Error;
use crate::error;

impl axum::response::IntoResponse for Error {
	fn into_response(self) -> axum::response::Response {
		let response: UiaaResponse = self.into();
		response
			.try_into_http_response::<BytesMut>()
			.inspect_err(|e| error!("error response error: {e}"))
			.map_or_else(
				|_| StatusCode::INTERNAL_SERVER_ERROR.into_response(),
				|r| r.map(BytesMut::freeze).map(Full::new).into_response(),
			)
	}
}

impl From<Error> for UiaaResponse {
	fn from(error: Error) -> Self {
		if let Error::Uiaa(uiaainfo) = error {
			return Self::AuthResponse(uiaainfo);
		}

		let body = ruma::api::client::error::ErrorBody::Standard {
			kind: error.kind(),
			message: error.message(),
		};

		Self::MatrixError(ruma::api::client::error::Error {
			status_code: error.status_code(),
			body,
		})
	}
}

pub(super) fn bad_request_code(kind: &ruma::api::client::error::ErrorKind) -> StatusCode {
	use ruma::api::client::error::ErrorKind::*;

	match kind {
		GuestAccessForbidden
		| ThreepidAuthFailed
		| UserDeactivated
		| ThreepidDenied
		| WrongRoomKeysVersion {
			..
		}
		| Forbidden {
			..
		} => StatusCode::FORBIDDEN,

		UnknownToken {
			..
		}
		| MissingToken
		| Unauthorized => StatusCode::UNAUTHORIZED,

		LimitExceeded {
			..
		} => StatusCode::TOO_MANY_REQUESTS,

		TooLarge => StatusCode::PAYLOAD_TOO_LARGE,

		NotFound | Unrecognized => StatusCode::NOT_FOUND,

		_ => StatusCode::BAD_REQUEST,
	}
}

pub(super) fn ruma_error_message(error: &ruma::api::client::error::Error) -> String {
	if let ruma::api::client::error::ErrorBody::Standard {
		message,
		..
	} = &error.body
	{
		return message.to_string();
	}

	format!("{error}")
}

pub(super) fn ruma_error_kind(e: &ruma::api::client::error::Error) -> &ruma::api::client::error::ErrorKind {
	e.error_kind()
		.unwrap_or(&ruma::api::client::error::ErrorKind::Unknown)
}
