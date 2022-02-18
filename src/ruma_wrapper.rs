use crate::Error;
use ruma::{
    api::client::uiaa::UiaaResponse,
    identifiers::{DeviceId, UserId},
    signatures::CanonicalJsonValue,
    Outgoing, ServerName,
};
use std::ops::Deref;

#[cfg(feature = "conduit_bin")]
mod axum;

/// Extractor for Ruma request structs
pub struct Ruma<T: Outgoing> {
    pub body: T::Incoming,
    pub sender_user: Option<Box<UserId>>,
    pub sender_device: Option<Box<DeviceId>>,
    pub sender_servername: Option<Box<ServerName>>,
    // This is None when body is not a valid string
    pub json_body: Option<CanonicalJsonValue>,
    pub from_appservice: bool,
}

impl<T: Outgoing> Deref for Ruma<T> {
    type Target = T::Incoming;

    fn deref(&self) -> &Self::Target {
        &self.body
    }
}

#[derive(Clone)]
pub struct RumaResponse<T>(pub T);

impl<T> From<T> for RumaResponse<T> {
    fn from(t: T) -> Self {
        Self(t)
    }
}

impl From<Error> for RumaResponse<UiaaResponse> {
    fn from(t: Error) -> Self {
        t.to_response()
    }
}
