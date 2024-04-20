use clap::Subcommand;
use ruma::{
	events::{room::message::RoomMessageEventContent, RoomAccountDataEventType},
	RoomId, UserId,
};

use crate::{services, Result};

#[cfg_attr(test, derive(Debug))]
#[derive(Subcommand)]
/// All the getters and iterators from src/database/key_value/account_data.rs
/// via services()
pub(crate) enum AccountData {
	/// - Returns all changes to the account data that happened after `since`.
	ChangesSince {
		/// Full user ID
		user_id: Box<UserId>,
		/// UNIX timestamp since (u64)
		since: u64,
		/// Optional room ID of the account data
		room_id: Option<Box<RoomId>>,
	},

	/// - Searches the account data for a specific kind.
	Get {
		/// Full user ID
		user_id: Box<UserId>,
		/// Account data event type
		kind: RoomAccountDataEventType,
		/// Optional room ID of the account data
		room_id: Option<Box<RoomId>>,
	},
}

/// All the getters and iterators in key_value/account_data.rs via services()
pub(crate) async fn account_data(subcommand: AccountData) -> Result<RoomMessageEventContent> {
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
