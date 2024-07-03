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
