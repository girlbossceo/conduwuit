use super::Count;

#[test]
fn backfilled_parse() {
	let count: Count = "-987654".parse().expect("parse() failed");
	let backfilled = matches!(count, Count::Backfilled(_));

	assert!(backfilled, "not backfilled variant");
}

#[test]
fn normal_parse() {
	let count: Count = "987654".parse().expect("parse() failed");
	let backfilled = matches!(count, Count::Backfilled(_));

	assert!(!backfilled, "backfilled variant");
}
