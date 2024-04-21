use ruma::events::room::message::RoomMessageEventContent;

use super::Sending;
use crate::{services, Result};

/// All the getters and iterators in key_value/sending.rs
pub(super) async fn sending(subcommand: Sending) -> Result<RoomMessageEventContent> {
	match subcommand {
		Sending::ActiveRequests => {
			let timer = tokio::time::Instant::now();
			let results = services().sending.db.active_requests();
			let query_time = timer.elapsed();

			let active_requests: Result<Vec<(_, _, _)>> = results.collect();

			Ok(RoomMessageEventContent::text_html(
				format!("Query completed in {query_time:?}:\n\n```\n{:?}```", active_requests),
				format!(
					"<p>Query completed in {query_time:?}:</p>\n<pre><code>{:?}\n</code></pre>",
					active_requests
				),
			))
		},
	}
}
