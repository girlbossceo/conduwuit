use std::time::{SystemTime, UNIX_EPOCH};

#[inline]
#[must_use]
#[allow(clippy::as_conversions)]
pub fn now_millis() -> u64 {
	UNIX_EPOCH
		.elapsed()
		.expect("positive duration after epoch")
		.as_millis() as u64
}

#[must_use]
pub fn rfc2822_from_seconds(epoch: i64) -> String {
	use chrono::{DateTime, Utc};

	DateTime::<Utc>::from_timestamp(epoch, 0)
		.unwrap_or_default()
		.to_rfc2822()
}

#[must_use]
pub fn format(ts: SystemTime, str: &str) -> String {
	use chrono::{DateTime, Utc};

	let dt: DateTime<Utc> = ts.into();
	dt.format(str).to_string()
}
