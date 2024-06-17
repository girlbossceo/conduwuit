use ruma::events::room::message::RoomMessageEventContent;

use super::Users;
use crate::{services, Result};

/// All the getters and iterators in key_value/users.rs
pub(super) async fn users(subcommand: Users) -> Result<RoomMessageEventContent> {
	match subcommand {
		Users::Iter => {
			let timer = tokio::time::Instant::now();
			let results = services().users.db.iter();
			let query_time = timer.elapsed();

			let users = results.collect::<Vec<_>>();

			Ok(RoomMessageEventContent::notice_markdown(format!(
				"Query completed in {query_time:?}:\n\n```rs\n{users:#?}\n```"
			)))
		},
	}
}
