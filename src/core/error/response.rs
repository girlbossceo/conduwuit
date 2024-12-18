use bytes::BytesMut;
use http::StatusCode;
use http_body_util::Full;
use ruma::api::{
	client::{
		error::{ErrorBody, ErrorKind},
		uiaa::UiaaResponse,
	},
	OutgoingResponse,
};

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
	#[inline]
	fn from(error: Error) -> Self {
		if let Error::Uiaa(uiaainfo) = error {
			return Self::AuthResponse(uiaainfo);
		}

		let body = ErrorBody::Standard {
			kind: error.kind(),
			message: error.message(),
		};

		Self::MatrixError(ruma::api::client::error::Error {
			status_code: error.status_code(),
			body,
		})
	}
}

pub(super) fn status_code(kind: &ErrorKind, hint: StatusCode) -> StatusCode {
	if hint == StatusCode::BAD_REQUEST {
		bad_request_code(kind)
	} else {
		hint
	}
}

pub(super) fn bad_request_code(kind: &ErrorKind) -> StatusCode {
	use ErrorKind::*;

	match kind {
		// 429
		| LimitExceeded { .. } => StatusCode::TOO_MANY_REQUESTS,

		// 413
		| TooLarge => StatusCode::PAYLOAD_TOO_LARGE,

		// 405
		| Unrecognized => StatusCode::METHOD_NOT_ALLOWED,

		// 404
		| NotFound | NotImplemented | FeatureDisabled => StatusCode::NOT_FOUND,

		// 403
		| GuestAccessForbidden
		| ThreepidAuthFailed
		| UserDeactivated
		| ThreepidDenied
		| WrongRoomKeysVersion { .. }
		| Forbidden { .. } => StatusCode::FORBIDDEN,

		// 401
		| UnknownToken { .. } | MissingToken | Unauthorized => StatusCode::UNAUTHORIZED,

		// 400
		| _ => StatusCode::BAD_REQUEST,
	}
}

pub(super) fn ruma_error_message(error: &ruma::api::client::error::Error) -> String {
	if let ErrorBody::Standard { message, .. } = &error.body {
		return message.to_string();
	}

	format!("{error}")
}

pub(super) fn ruma_error_kind(e: &ruma::api::client::error::Error) -> &ErrorKind {
	e.error_kind().unwrap_or(&ErrorKind::Unknown)
}
