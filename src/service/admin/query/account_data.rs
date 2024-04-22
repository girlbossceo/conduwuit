use ruma::events::room::message::RoomMessageEventContent;

use super::AccountData;
use crate::{services, Result};

/// All the getters and iterators from src/database/key_value/account_data.rs
pub(super) async fn account_data(subcommand: AccountData) -> Result<RoomMessageEventContent> {
	match subcommand {
		AccountData::ChangesSince {
			user_id,
			since,
			room_id,
		} => {
			let timer = tokio::time::Instant::now();
			let results = services()
				.account_data
				.db
				.changes_since(room_id.as_deref(), &user_id, since)?;
			let query_time = timer.elapsed();

			Ok(RoomMessageEventContent::text_html(
				format!("Query completed in {query_time:?}:\n\n```\n{:?}```", results),
				format!(
					"<p>Query completed in {query_time:?}:</p>\n<pre><code>{:?}\n</code></pre>",
					results
				),
			))
		},
		AccountData::Get {
			user_id,
			kind,
			room_id,
		} => {
			let timer = tokio::time::Instant::now();
			let results = services()
				.account_data
				.db
				.get(room_id.as_deref(), &user_id, kind)?;
			let query_time = timer.elapsed();

			Ok(RoomMessageEventContent::text_html(
				format!("Query completed in {query_time:?}:\n\n```\n{:?}```", results),
				format!(
					"<p>Query completed in {query_time:?}:</p>\n<pre><code>{:?}\n</code></pre>",
					results
				),
			))
		},
	}
}
