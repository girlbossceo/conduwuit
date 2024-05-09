use ruma::events::room::message::RoomMessageEventContent;

use super::Appservice;
use crate::{services, Result};

/// All the getters and iterators from src/database/key_value/appservice.rs
pub(crate) async fn appservice(subcommand: Appservice) -> Result<RoomMessageEventContent> {
	match subcommand {
		Appservice::GetRegistration {
			appservice_id,
		} => {
			let timer = tokio::time::Instant::now();
			let results = services()
				.appservice
				.db
				.get_registration(appservice_id.as_ref());
			let query_time = timer.elapsed();

			Ok(RoomMessageEventContent::text_html(
				format!("Query completed in {query_time:?}:\n\n```\n{:?}```", results),
				format!(
					"<p>Query completed in {query_time:?}:</p>\n<pre><code>{:?}\n</code></pre>",
					results
				),
			))
		},
		Appservice::All => {
			let timer = tokio::time::Instant::now();
			let results = services().appservice.db.all();
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
