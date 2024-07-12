use std::{
	any::Any,
	convert::Infallible,
	fmt,
	panic::{RefUnwindSafe, UnwindSafe},
};

use bytes::BytesMut;
use http::StatusCode;
use http_body_util::Full;
use ruma::{
	api::{client::uiaa::UiaaResponse, OutgoingResponse},
	OwnedServerName,
};

use crate::{debug::panic_str, debug_error, error};

#[macro_export]
macro_rules! err {
	(error!($($args:tt),+)) => {{
		error!($($args),+);
		$crate::error::Error::Err(format!($($args),+))
	}};

	(debug_error!($($args:tt),+)) => {{
		debug_error!($($args),+);
		$crate::error::Error::Err(format!($($args),+))
	}};

	($variant:ident(error!($($args:tt),+))) => {{
		error!($($args),+);
		$crate::error::Error::$variant(format!($($args),+))
	}};

	($variant:ident(debug_error!($($args:tt),+))) => {{
		debug_error!($($args),+);
		$crate::error::Error::$variant(format!($($args),+))
	}};

	($variant:ident(format!($($args:tt),+))) => {
		$crate::error::Error::$variant(format!($($args),+))
	};

	($variant:ident($($args:tt),+)) => {
		$crate::error::Error::$variant(format!($($args),+))
	};

	($string:literal$(,)? $($args:tt),*) => {
		$crate::error::Error::Err(format!($string, $($args),*))
	};
}

#[macro_export]
macro_rules! Err {
	($($args:tt)*) => {
		Err($crate::err!($($args)*))
	};
}

#[derive(thiserror::Error)]
pub enum Error {
	#[error("PANIC!")]
	PanicAny(Box<dyn Any + Send>),
	#[error("PANIC! {0}")]
	Panic(&'static str, Box<dyn Any + Send + 'static>),

	// std
	#[error("{0}")]
	Fmt(#[from] fmt::Error),
	#[error("I/O error: {0}")]
	Io(#[from] std::io::Error),
	#[error("{0}")]
	Utf8Error(#[from] std::str::Utf8Error),
	#[error("{0}")]
	FromUtf8Error(#[from] std::string::FromUtf8Error),
	#[error("{0}")]
	TryFromSliceError(#[from] std::array::TryFromSliceError),
	#[error("{0}")]
	TryFromIntError(#[from] std::num::TryFromIntError),
	#[error("{0}")]
	ParseIntError(#[from] std::num::ParseIntError),
	#[error("{0}")]
	ParseFloatError(#[from] std::num::ParseFloatError),

	// third-party
	#[error("Join error: {0}")]
	JoinError(#[from] tokio::task::JoinError),
	#[error("Regex error: {0}")]
	Regex(#[from] regex::Error),
	#[error("Tracing filter error: {0}")]
	TracingFilter(#[from] tracing_subscriber::filter::ParseError),
	#[error("Tracing reload error: {0}")]
	TracingReload(#[from] tracing_subscriber::reload::Error),
	#[error("Image error: {0}")]
	Image(#[from] image::error::ImageError),
	#[error("Request error: {0}")]
	Reqwest(#[from] reqwest::Error),
	#[error("{0}")]
	Extension(#[from] axum::extract::rejection::ExtensionRejection),
	#[error("{0}")]
	Path(#[from] axum::extract::rejection::PathRejection),
	#[error("{0}")]
	Http(#[from] http::Error),

	// ruma
	#[error("{0}")]
	IntoHttpError(#[from] ruma::api::error::IntoHttpError),
	#[error("{0}")]
	RumaError(#[from] ruma::api::client::error::Error),
	#[error("uiaa")]
	Uiaa(ruma::api::client::uiaa::UiaaInfo),
	#[error("{0}")]
	Mxid(#[from] ruma::IdParseError),
	#[error("{0}: {1}")]
	BadRequest(ruma::api::client::error::ErrorKind, &'static str),
	#[error("from {0}: {1}")]
	Redaction(OwnedServerName, ruma::canonical_json::RedactionError),
	#[error("Remote server {0} responded with: {1}")]
	Federation(OwnedServerName, ruma::api::client::error::Error),
	#[error("{0} in {1}")]
	InconsistentRoomState(&'static str, ruma::OwnedRoomId),

	// conduwuit
	#[error("Arithmetic operation failed: {0}")]
	Arithmetic(&'static str),
	#[error("There was a problem with your configuration: {0}")]
	BadConfig(String),
	#[error("{0}")]
	BadDatabase(&'static str),
	#[error("{0}")]
	Database(String),
	#[error("{0}")]
	BadServerResponse(&'static str),
	#[error("{0}")]
	Conflict(&'static str), // This is only needed for when a room alias already exists

	// unique / untyped
	#[error("{0}")]
	Err(String),
}

impl Error {
	pub fn bad_database(message: &'static str) -> Self {
		error!("BadDatabase: {}", message);
		Self::BadDatabase(message)
	}

	pub fn bad_config(message: &str) -> Self {
		error!("BadConfig: {}", message);
		Self::BadConfig(message.to_owned())
	}

	#[must_use]
	pub fn from_panic(e: Box<dyn Any + Send>) -> Self { Self::Panic(panic_str(&e), e) }

	pub fn into_panic(self) -> Box<dyn Any + Send + 'static> {
		match self {
			Self::Panic(_, e) | Self::PanicAny(e) => e,
			Self::JoinError(e) => e.into_panic(),
			_ => std::panic::panic_any(self),
		}
	}

	/// Sanitizes public-facing errors that can leak sensitive information.
	pub fn sanitized_string(&self) -> String {
		match self {
			Self::Database(..) => String::from("Database error occurred."),
			Self::Io(..) => String::from("I/O error occurred."),
			_ => self.to_string(),
		}
	}

	/// Get the panic message string.
	pub fn panic_str(self) -> Option<&'static str> { self.is_panic().then_some(panic_str(&self.into_panic())) }

	/// Returns the Matrix error code / error kind
	#[inline]
	pub fn error_code(&self) -> ruma::api::client::error::ErrorKind {
		use ruma::api::client::error::ErrorKind::Unknown;

		match self {
			Self::Federation(_, error) => ruma_error_kind(error).clone(),
			Self::BadRequest(kind, _) => kind.clone(),
			_ => Unknown,
		}
	}

	/// Check if the Error is trafficking a panic object.
	#[inline]
	pub fn is_panic(&self) -> bool {
		match &self {
			Self::Panic(..) | Self::PanicAny(..) => true,
			Self::JoinError(e) => e.is_panic(),
			_ => false,
		}
	}
}

impl UnwindSafe for Error {}
impl RefUnwindSafe for Error {}

impl From<Infallible> for Error {
	fn from(i: Infallible) -> Self { match i {} }
}

impl fmt::Debug for Error {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "{self}") }
}

#[inline]
pub fn else_log<T, E>(error: E) -> Result<T, Infallible>
where
	T: Default,
	Error: From<E>,
{
	Ok(default_log(error))
}

#[inline]
pub fn else_debug_log<T, E>(error: E) -> Result<T, Infallible>
where
	T: Default,
	Error: From<E>,
{
	Ok(default_debug_log(error))
}

#[inline]
pub fn default_log<T, E>(error: E) -> T
where
	T: Default,
	Error: From<E>,
{
	let error = Error::from(error);
	inspect_log(&error);
	T::default()
}

#[inline]
pub fn default_debug_log<T, E>(error: E) -> T
where
	T: Default,
	Error: From<E>,
{
	let error = Error::from(error);
	inspect_debug_log(&error);
	T::default()
}

#[inline]
pub fn map_log<E>(error: E) -> Error
where
	Error: From<E>,
{
	let error = Error::from(error);
	inspect_log(&error);
	error
}

#[inline]
pub fn map_debug_log<E>(error: E) -> Error
where
	Error: From<E>,
{
	let error = Error::from(error);
	inspect_debug_log(&error);
	error
}

#[inline]
pub fn inspect_log<E: fmt::Display>(error: &E) {
	error!("{error}");
}

#[inline]
pub fn inspect_debug_log<E: fmt::Debug>(error: &E) {
	debug_error!("{error:?}");
}

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

		let kind = match &error {
			Error::Federation(_, ref error) | Error::RumaError(ref error) => ruma_error_kind(error),
			Error::BadRequest(kind, _) => kind,
			_ => &ruma::api::client::error::ErrorKind::Unknown,
		};

		let status_code = match &error {
			Error::Federation(_, ref error) | Error::RumaError(ref error) => error.status_code,
			Error::BadRequest(ref kind, _) => bad_request_code(kind),
			Error::Conflict(_) => StatusCode::CONFLICT,
			_ => StatusCode::INTERNAL_SERVER_ERROR,
		};

		let message = match &error {
			Error::Federation(ref origin, ref error) => format!("Answer from {origin}: {error}"),
			Error::RumaError(ref error) => ruma_error_message(error),
			_ => format!("{error}"),
		};

		let body = ruma::api::client::error::ErrorBody::Standard {
			kind: kind.clone(),
			message,
		};

		Self::MatrixError(ruma::api::client::error::Error {
			status_code,
			body,
		})
	}
}

fn bad_request_code(kind: &ruma::api::client::error::ErrorKind) -> StatusCode {
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

fn ruma_error_message(error: &ruma::api::client::error::Error) -> String {
	if let ruma::api::client::error::ErrorBody::Standard {
		message,
		..
	} = &error.body
	{
		return message.to_string();
	}

	format!("{error}")
}

fn ruma_error_kind(e: &ruma::api::client::error::Error) -> &ruma::api::client::error::ErrorKind {
	e.error_kind()
		.unwrap_or(&ruma::api::client::error::ErrorKind::Unknown)
}
