#![cfg(test)]

use super::PduCount;

#[test]
fn backfilled_parse() {
	let count: PduCount = "-987654".parse().expect("parse() failed");
	let backfilled = matches!(count, PduCount::Backfilled(_));

	assert!(backfilled, "not backfilled variant");
}

#[test]
fn normal_parse() {
	let count: PduCount = "987654".parse().expect("parse() failed");
	let backfilled = matches!(count, PduCount::Backfilled(_));

	assert!(!backfilled, "backfilled variant");
}
