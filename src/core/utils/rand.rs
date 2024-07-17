use std::{
	ops::Range,
	time::{Duration, SystemTime},
};

use rand::{thread_rng, Rng};

pub fn string(length: usize) -> String {
	thread_rng()
		.sample_iter(&rand::distributions::Alphanumeric)
		.take(length)
		.map(char::from)
		.collect()
}

#[inline]
#[must_use]
pub fn timepoint_secs(range: Range<u64>) -> SystemTime {
	SystemTime::now()
		.checked_add(secs(range))
		.expect("range does not overflow SystemTime")
}

#[must_use]
pub fn secs(range: Range<u64>) -> Duration {
	let mut rng = thread_rng();
	Duration::from_secs(rng.gen_range(range))
}
