use ruma::events::room::message::RoomMessageEventContent;

use super::Globals;
use crate::{services, Result};

/// All the getters and iterators from src/database/key_value/globals.rs
pub(super) async fn globals(subcommand: Globals) -> Result<RoomMessageEventContent> {
	match subcommand {
		Globals::DatabaseVersion => {
			let timer = tokio::time::Instant::now();
			let results = services().globals.db.database_version();
			let query_time = timer.elapsed();

			Ok(RoomMessageEventContent::text_html(
				format!("Query completed in {query_time:?}:\n\n```\n{:?}```", results),
				format!(
					"<p>Query completed in {query_time:?}:</p>\n<pre><code>{:?}\n</code></pre>",
					results
				),
			))
		},
		Globals::CurrentCount => {
			let timer = tokio::time::Instant::now();
			let results = services().globals.db.current_count();
			let query_time = timer.elapsed();

			Ok(RoomMessageEventContent::text_html(
				format!("Query completed in {query_time:?}:\n\n```\n{:?}```", results),
				format!(
					"<p>Query completed in {query_time:?}:</p>\n<pre><code>{:?}\n</code></pre>",
					results
				),
			))
		},
		Globals::LastCheckForUpdatesId => {
			let timer = tokio::time::Instant::now();
			let results = services().globals.db.last_check_for_updates_id();
			let query_time = timer.elapsed();

			Ok(RoomMessageEventContent::text_html(
				format!("Query completed in {query_time:?}:\n\n```\n{:?}```", results),
				format!(
					"<p>Query completed in {query_time:?}:</p>\n<pre><code>{:?}\n</code></pre>",
					results
				),
			))
		},
		Globals::LoadKeypair => {
			let timer = tokio::time::Instant::now();
			let results = services().globals.db.load_keypair();
			let query_time = timer.elapsed();

			Ok(RoomMessageEventContent::text_html(
				format!("Query completed in {query_time:?}:\n\n```\n{:?}```", results),
				format!(
					"<p>Query completed in {query_time:?}:</p>\n<pre><code>{:?}\n</code></pre>",
					results
				),
			))
		},
		Globals::SigningKeysFor {
			origin,
		} => {
			let timer = tokio::time::Instant::now();
			let results = services().globals.db.signing_keys_for(&origin);
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
