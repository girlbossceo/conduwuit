use std::convert::Infallible;

use http::StatusCode;
use ruma::{
	api::client::{
		error::{Error as RumaError, ErrorBody, ErrorKind},
		uiaa::{UiaaInfo, UiaaResponse},
	},
	OwnedServerName,
};
use thiserror::Error;
use tracing::{error, info};
use ErrorKind::{
	Forbidden, GuestAccessForbidden, LimitExceeded, MissingToken, NotFound, ThreepidAuthFailed, ThreepidDenied,
	TooLarge, Unauthorized, Unknown, UnknownToken, Unrecognized, UserDeactivated, WrongRoomKeysVersion,
};

use crate::RumaResponse;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Error, Debug)]
pub enum Error {
	#[cfg(feature = "sqlite")]
	#[error("There was a problem with the connection to the sqlite database: {source}")]
	SqliteError {
		#[from]
		source: rusqlite::Error,
	},
	#[cfg(feature = "rocksdb")]
	#[error("There was a problem with the connection to the rocksdb database: {source}")]
	RocksDbError {
		#[from]
		source: rust_rocksdb::Error,
	},
	#[error("Could not generate an image.")]
	ImageError {
		#[from]
		source: image::error::ImageError,
	},
	#[error("Could not connect to server: {source}")]
	ReqwestError {
		#[from]
		source: reqwest::Error,
	},
	#[error("Could build regular expression: {source}")]
	RegexError {
		#[from]
		source: regex::Error,
	},
	#[error("{0}")]
	FederationError(OwnedServerName, RumaError),
	#[error("Could not do this io: {source}")]
	IoError {
		#[from]
		source: std::io::Error,
	},
	#[error("{0}")]
	BadServerResponse(&'static str),
	#[error("{0}")]
	BadConfig(&'static str),
	#[error("{0}")]
	/// Don't create this directly. Use Error::bad_database instead.
	BadDatabase(&'static str),
	#[error("uiaa")]
	Uiaa(UiaaInfo),
	#[error("{0}: {1}")]
	BadRequest(ErrorKind, &'static str),
	#[error("{0}")]
	Conflict(&'static str), // This is only needed for when a room alias already exists
	#[error("{0}")]
	ExtensionError(#[from] axum::extract::rejection::ExtensionRejection),
	#[error("{0}")]
	PathError(#[from] axum::extract::rejection::PathRejection),
	#[error("from {0}: {1}")]
	RedactionError(OwnedServerName, ruma::canonical_json::RedactionError),
	#[error("{0} in {1}")]
	InconsistentRoomState(&'static str, ruma::OwnedRoomId),
	#[error("{0}")]
	AdminCommand(&'static str),
}

impl Error {
	pub fn bad_database(message: &'static str) -> Self {
		error!("BadDatabase: {}", message);
		Self::BadDatabase(message)
	}

	pub fn bad_config(message: &'static str) -> Self {
		error!("BadConfig: {}", message);
		Self::BadConfig(message)
	}
}

impl Error {
	pub fn to_response(&self) -> RumaResponse<UiaaResponse> {
		if let Self::Uiaa(uiaainfo) = self {
			return RumaResponse(UiaaResponse::AuthResponse(uiaainfo.clone()));
		}

		if let Self::FederationError(origin, error) = self {
			let mut error = error.clone();
			error.body = ErrorBody::Standard {
				kind: error.error_kind().unwrap_or(&Unknown).clone(),
				message: format!("Answer from {origin}: {error}"),
			};
			return RumaResponse(UiaaResponse::MatrixError(error));
		}

		let message = format!("{self}");

		let (kind, status_code) = match self {
			Self::BadRequest(kind, _) => (
				kind.clone(),
				match kind {
					WrongRoomKeysVersion {
						..
					}
					| Forbidden
					| GuestAccessForbidden
					| ThreepidAuthFailed
					| UserDeactivated
					| ThreepidDenied => StatusCode::FORBIDDEN,
					Unauthorized
					| UnknownToken {
						..
					}
					| MissingToken => StatusCode::UNAUTHORIZED,
					NotFound | Unrecognized => StatusCode::NOT_FOUND,
					LimitExceeded {
						..
					} => StatusCode::TOO_MANY_REQUESTS,
					TooLarge => StatusCode::PAYLOAD_TOO_LARGE,
					_ => StatusCode::BAD_REQUEST,
				},
			),
			Self::Conflict(_) => (Unknown, StatusCode::CONFLICT),
			_ => (Unknown, StatusCode::INTERNAL_SERVER_ERROR),
		};

		info!("Returning an error: {status_code}: {message}");

		RumaResponse(UiaaResponse::MatrixError(RumaError {
			body: ErrorBody::Standard {
				kind,
				message,
			},
			status_code,
		}))
	}

	/// Returns the Matrix error code / error kind
	pub fn error_code(&self) -> ErrorKind {
		if let Self::FederationError(_, error) = self {
			return error.error_kind().unwrap_or(&Unknown).clone();
		}

		match self {
			Self::BadRequest(kind, _) => kind.clone(),
			_ => Unknown,
		}
	}

	/// Sanitizes public-facing errors that can leak sensitive information.
	pub fn sanitized_error(&self) -> String {
		let db_error = String::from("Database or I/O error occurred.");

		match self {
			#[cfg(feature = "sqlite")]
			Self::SqliteError {
				..
			} => db_error,
			#[cfg(feature = "rocksdb")]
			Self::RocksDbError {
				..
			} => db_error,
			Self::IoError {
				..
			} => db_error,
			Self::BadConfig {
				..
			} => db_error,
			Self::BadDatabase {
				..
			} => db_error,
			_ => self.to_string(),
		}
	}
}

impl From<Infallible> for Error {
	fn from(i: Infallible) -> Self { match i {} }
}

impl axum::response::IntoResponse for Error {
	fn into_response(self) -> axum::response::Response { self.to_response().into_response() }
}
