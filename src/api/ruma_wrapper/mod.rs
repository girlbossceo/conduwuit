use std::ops::Deref;

use ruma::{api::client::uiaa::UiaaResponse, CanonicalJsonValue, OwnedDeviceId, OwnedServerName, OwnedUserId};

use crate::{service::appservice::RegistrationInfo, Error};

mod axum;

/// Extractor for Ruma request structs
pub(crate) struct Ruma<T> {
	pub(crate) body: T,
	pub(crate) sender_user: Option<OwnedUserId>,
	pub(crate) sender_device: Option<OwnedDeviceId>,
	pub(crate) sender_servername: Option<OwnedServerName>,
	pub(crate) json_body: Option<CanonicalJsonValue>, // This is None when body is not a valid string
	pub(crate) appservice_info: Option<RegistrationInfo>,
}

impl<T> Deref for Ruma<T> {
	type Target = T;

	fn deref(&self) -> &Self::Target { &self.body }
}

#[derive(Clone)]
pub(crate) struct RumaResponse<T>(pub(crate) T);

impl<T> From<T> for RumaResponse<T> {
	fn from(t: T) -> Self { Self(t) }
}

impl From<Error> for RumaResponse<UiaaResponse> {
	fn from(t: Error) -> Self { t.to_response() }
}
