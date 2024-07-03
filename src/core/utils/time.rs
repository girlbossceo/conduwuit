use std::time::{SystemTime, UNIX_EPOCH};

#[inline]
#[must_use]
#[allow(clippy::as_conversions)]
pub fn millis_since_unix_epoch() -> u64 {
	SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.expect("time is valid")
		.as_millis() as u64
}

#[must_use]
pub fn rfc2822_from_seconds(epoch: i64) -> String {
	use chrono::{DateTime, Utc};

	DateTime::<Utc>::from_timestamp(epoch, 0)
		.unwrap_or_default()
		.to_rfc2822()
}
