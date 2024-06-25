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
