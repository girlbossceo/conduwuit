use tracing::Level;
use tracing_core::{span::Current, Event};

use super::layer::Value;

pub struct Data<'a> {
	pub event: &'a Event<'a>,
	pub current: &'a Current,
	pub values: Option<&'a mut [Value]>,
}

impl Data<'_> {
	#[must_use]
	pub fn level(&self) -> Level { *self.event.metadata().level() }

	#[must_use]
	pub fn mod_name(&self) -> &str { self.event.metadata().module_path().unwrap_or_default() }

	#[must_use]
	pub fn span_name(&self) -> &str { self.current.metadata().map_or("", |s| s.name()) }

	#[must_use]
	pub fn message(&self) -> &str {
		self.values
			.as_ref()
			.expect("values are not composed for a filter")
			.iter()
			.find(|(k, _)| *k == "message")
			.map_or("", |(_, v)| v.as_str())
	}
}
