use std::str;

use axum_extra::headers::authorization::Credentials;
use ruma::OwnedServerName;
use tracing::debug;

pub(crate) struct XMatrix {
	pub(crate) origin: OwnedServerName,
	pub(crate) destination: Option<String>,
	pub(crate) key: String, // KeyName?
	pub(crate) sig: String,
}

impl Credentials for XMatrix {
	const SCHEME: &'static str = "X-Matrix";

	fn decode(value: &http::HeaderValue) -> Option<Self> {
		debug_assert!(
			value.as_bytes().starts_with(b"X-Matrix "),
			"HeaderValue to decode should start with \"X-Matrix ..\", received = {value:?}",
		);

		let parameters = str::from_utf8(&value.as_bytes()["X-Matrix ".len()..])
			.ok()?
			.trim_start();

		let mut origin = None;
		let mut destination = None;
		let mut key = None;
		let mut sig = None;

		for entry in parameters.split_terminator(',') {
			let (name, value) = entry.split_once('=')?;

			// It's not at all clear why some fields are quoted and others not in the spec,
			// let's simply accept either form for every field.
			let value = value
				.strip_prefix('"')
				.and_then(|rest| rest.strip_suffix('"'))
				.unwrap_or(value);

			// FIXME: Catch multiple fields of the same name
			match name {
				"origin" => origin = Some(value.try_into().ok()?),
				"key" => key = Some(value.to_owned()),
				"sig" => sig = Some(value.to_owned()),
				"destination" => destination = Some(value.to_owned()),
				_ => debug!("Unexpected field `{name}` in X-Matrix Authorization header"),
			}
		}

		Some(Self {
			origin: origin?,
			key: key?,
			sig: sig?,
			destination,
		})
	}

	fn encode(&self) -> http::HeaderValue { todo!() }
}
