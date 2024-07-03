#[inline]
#[must_use]
#[allow(clippy::as_conversions)]
pub fn now_millis() -> u64 {
	use std::time::UNIX_EPOCH;

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
