#![cfg(test)]

use crate::utils;

#[test]
fn increment_none() {
	let bytes: [u8; 8] = utils::increment(None);
	let res = u64::from_be_bytes(bytes);
	assert_eq!(res, 1);
}

#[test]
fn increment_fault() {
	let start: u8 = 127;
	let bytes: [u8; 1] = start.to_be_bytes();
	let bytes: [u8; 8] = utils::increment(Some(&bytes));
	let res = u64::from_be_bytes(bytes);
	assert_eq!(res, 1);
}

#[test]
fn increment_norm() {
	let start: u64 = 1_234_567;
	let bytes: [u8; 8] = start.to_be_bytes();
	let bytes: [u8; 8] = utils::increment(Some(&bytes));
	let res = u64::from_be_bytes(bytes);
	assert_eq!(res, 1_234_568);
}

#[test]
fn increment_wrap() {
	let start = u64::MAX;
	let bytes: [u8; 8] = start.to_be_bytes();
	let bytes: [u8; 8] = utils::increment(Some(&bytes));
	let res = u64::from_be_bytes(bytes);
	assert_eq!(res, 0);
}

#[test]
fn checked_add() {
	use crate::checked;

	let a = 1234;
	let res = checked!(a + 1).unwrap();
	assert_eq!(res, 1235);
}

#[test]
#[should_panic(expected = "overflow")]
fn checked_add_overflow() {
	use crate::checked;

	let a = u64::MAX;
	let res = checked!(a + 1).expect("overflow");
	assert_eq!(res, 0);
}

#[tokio::test]
async fn mutex_map_cleanup() {
	use crate::utils::MutexMap;

	let map = MutexMap::<String, ()>::new();

	let lock = map.lock("foo").await;
	assert!(!map.is_empty(), "map must not be empty");

	drop(lock);
	assert!(map.is_empty(), "map must be empty");
}

#[tokio::test]
async fn mutex_map_contend() {
	use std::sync::Arc;

	use tokio::sync::Barrier;

	use crate::utils::MutexMap;

	let map = Arc::new(MutexMap::<String, ()>::new());
	let seq = Arc::new([Barrier::new(2), Barrier::new(2)]);
	let str = "foo".to_owned();

	let seq_ = seq.clone();
	let map_ = map.clone();
	let str_ = str.clone();
	let join_a = tokio::spawn(async move {
		let _lock = map_.lock(&str_).await;
		assert!(!map_.is_empty(), "A0 must not be empty");
		seq_[0].wait().await;
		assert!(map_.contains(&str_), "A1 must contain key");
	});

	let seq_ = seq.clone();
	let map_ = map.clone();
	let str_ = str.clone();
	let join_b = tokio::spawn(async move {
		let _lock = map_.lock(&str_).await;
		assert!(!map_.is_empty(), "B0 must not be empty");
		seq_[1].wait().await;
		assert!(map_.contains(&str_), "B1 must contain key");
	});

	seq[0].wait().await;
	assert!(map.contains(&str), "Must contain key");
	seq[1].wait().await;

	tokio::try_join!(join_b, join_a).expect("joined");
	assert!(map.is_empty(), "Must be empty");
}
