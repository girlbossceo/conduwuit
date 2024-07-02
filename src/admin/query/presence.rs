use ruma::events::room::message::RoomMessageEventContent;

use super::Presence;
use crate::{services, Result};

/// All the getters and iterators in key_value/presence.rs
pub(super) async fn presence(subcommand: Presence) -> Result<RoomMessageEventContent> {
	match subcommand {
		Presence::GetPresence {
			user_id,
		} => {
			let timer = tokio::time::Instant::now();
			let results = services().presence.db.get_presence(&user_id)?;
			let query_time = timer.elapsed();

			Ok(RoomMessageEventContent::notice_markdown(format!(
				"Query completed in {query_time:?}:\n\n```rs\n{results:#?}\n```"
			)))
		},
		Presence::PresenceSince {
			since,
		} => {
			let timer = tokio::time::Instant::now();
			let results = services().presence.db.presence_since(since);
			let presence_since: Vec<(_, _, _)> = results.collect();
			let query_time = timer.elapsed();

			Ok(RoomMessageEventContent::notice_markdown(format!(
				"Query completed in {query_time:?}:\n\n```rs\n{presence_since:#?}\n```"
			)))
		},
	}
}
