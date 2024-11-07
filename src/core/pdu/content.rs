use serde::Deserialize;
use serde_json::value::Value as JsonValue;

use crate::{err, implement, Result};

#[must_use]
#[implement(super::Pdu)]
pub fn get_content_as_value(&self) -> JsonValue {
	self.get_content()
		.expect("pdu content must be a valid JSON value")
}

#[implement(super::Pdu)]
pub fn get_content<T>(&self) -> Result<T>
where
	T: for<'de> Deserialize<'de>,
{
	serde_json::from_str(self.content.get())
		.map_err(|e| err!(Database("Failed to deserialize pdu content into type: {e}")))
}
