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
fn common_prefix() {
	use utils::string;

	let input = ["conduwuit", "conduit", "construct"];
	let output = string::common_prefix(&input);
	assert_eq!(output, "con");
}

#[test]
fn common_prefix_empty() {
	use utils::string;

	let input = ["abcdefg", "hijklmn", "opqrstu"];
	let output = string::common_prefix(&input);
	assert_eq!(output, "");
}

#[test]
fn common_prefix_none() {
	use utils::string;

	let input = [];
	let output = string::common_prefix(&input);
	assert_eq!(output, "");
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
