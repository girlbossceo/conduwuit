use conduit::Result;
use conduit_macros::implement;
use ruma::events::room::message::RoomMessageEventContent;

use crate::{services, Command};

/// Uses the iterator in `src/database/key_value/users.rs` to iterator over
/// every user in our database (remote and local). Reports total count, any
/// errors if there were any, etc
#[implement(Command, params = "<'_>")]
pub(super) async fn check_all_users(&self) -> Result<RoomMessageEventContent> {
	let timer = tokio::time::Instant::now();
	let results = services().users.db.iter();
	let query_time = timer.elapsed();

	let users = results.collect::<Vec<_>>();

	let total = users.len();
	let err_count = users.iter().filter(|user| user.is_err()).count();
	let ok_count = users.iter().filter(|user| user.is_ok()).count();

	let message = format!(
		"Database query completed in {query_time:?}:\n\n```\nTotal entries: {total:?}\nFailure/Invalid user count: \
		 {err_count:?}\nSuccess/Valid user count: {ok_count:?}\n```"
	);

	Ok(RoomMessageEventContent::notice_markdown(message))
}
