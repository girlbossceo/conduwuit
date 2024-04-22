use ruma::events::room::message::RoomMessageEventContent;

use crate::{services, Result};

/// Uses the iterator in `src/database/key_value/users.rs` to iterator over
/// every user in our database (remote and local). Reports total count, any
/// errors if there were any, etc
pub(super) async fn check_all_users(_body: Vec<&str>) -> Result<RoomMessageEventContent> {
	let timer = tokio::time::Instant::now();
	let results = services().users.db.iter();
	let query_time = timer.elapsed();

	let users = results.collect::<Vec<_>>();

	let total = users.len();
	let err_count = users.iter().filter(|user| user.is_err()).count();
	let ok_count = users.iter().filter(|user| user.is_ok()).count();

	let message = format!(
		"Database query completed in {query_time:?}:\n\n```\nTotal entries: {:?}\nFailure/Invalid user count: \
		 {:?}\nSuccess/Valid user count: {:?}```",
		total, err_count, ok_count
	);

	Ok(RoomMessageEventContent::notice_html(message, String::new()))
}
