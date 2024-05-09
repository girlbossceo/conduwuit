pub(crate) mod axum;
mod xmatrix;

use std::ops::Deref;

use ruma::{CanonicalJsonValue, OwnedDeviceId, OwnedServerName, OwnedUserId};

use crate::service::appservice::RegistrationInfo;

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
