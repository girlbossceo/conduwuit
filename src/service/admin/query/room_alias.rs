use clap::Subcommand;
use ruma::{events::room::message::RoomMessageEventContent, RoomAliasId, RoomId};

use crate::{services, Result};

#[cfg_attr(test, derive(Debug))]
#[derive(Subcommand)]
/// All the getters and iterators from src/database/key_value/rooms/alias.rs
pub(crate) enum RoomAlias {
	ResolveLocalAlias {
		/// Full room alias
		alias: Box<RoomAliasId>,
	},

	/// - Iterator of all our local room aliases for the room ID
	LocalAliasesForRoom {
		/// Full room ID
		room_id: Box<RoomId>,
	},

	/// - Iterator of all our local aliases in our database with their room IDs
	AllLocalAliases,
}

/// All the getters and iterators in src/database/key_value/rooms/alias.rs
pub(super) async fn room_alias(subcommand: RoomAlias) -> Result<RoomMessageEventContent> {
	match subcommand {
		RoomAlias::ResolveLocalAlias {
			alias,
		} => {
			let timer = tokio::time::Instant::now();
			let results = services().rooms.alias.db.resolve_local_alias(&alias);
			let query_time = timer.elapsed();

			Ok(RoomMessageEventContent::text_html(
				format!("Query completed in {query_time:?}:\n\n```\n{:?}```", results),
				format!(
					"<p>Query completed in {query_time:?}:</p>\n<pre><code>{:?}\n</code></pre>",
					results
				),
			))
		},
		RoomAlias::LocalAliasesForRoom {
			room_id,
		} => {
			let timer = tokio::time::Instant::now();
			let results = services().rooms.alias.db.local_aliases_for_room(&room_id);
			let query_time = timer.elapsed();

			let aliases: Vec<_> = results.collect();

			Ok(RoomMessageEventContent::text_html(
				format!("Query completed in {query_time:?}:\n\n```\n{:?}```", aliases),
				format!(
					"<p>Query completed in {query_time:?}:</p>\n<pre><code>{:?}\n</code></pre>",
					aliases
				),
			))
		},
		RoomAlias::AllLocalAliases => {
			let timer = tokio::time::Instant::now();
			let results = services().rooms.alias.db.all_local_aliases();
			let query_time = timer.elapsed();

			let aliases: Vec<_> = results.collect();

			Ok(RoomMessageEventContent::text_html(
				format!("Query completed in {query_time:?}:\n\n```\n{:?}```", aliases),
				format!(
					"<p>Query completed in {query_time:?}:</p>\n<pre><code>{:?}\n</code></pre>",
					aliases
				),
			))
		},
	}
}
