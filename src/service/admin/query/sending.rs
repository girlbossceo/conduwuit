use ruma::events::room::message::RoomMessageEventContent;

use super::Sending;
use crate::{service::sending::Destination, services, Result};

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
		Sending::QueuedRequests {
			appservice_id,
			server_name,
			user_id,
			push_key,
		} => {
			if appservice_id.is_none() && server_name.is_none() && user_id.is_none() && push_key.is_none() {
				return Ok(RoomMessageEventContent::text_plain(
					"An appservice ID, server name, or a user ID with push key must be specified via arguments. See \
					 --help for more details.",
				));
			}

			let (results, query_time) = match (appservice_id, server_name, user_id, push_key) {
				(Some(appservice_id), None, None, None) => {
					if appservice_id.is_empty() {
						return Ok(RoomMessageEventContent::text_plain(
							"An appservice ID, server name, or a user ID with push key must be specified via \
							 arguments. See --help for more details.",
						));
					}

					let timer = tokio::time::Instant::now();
					let results = services()
						.sending
						.db
						.queued_requests(&Destination::Appservice(appservice_id));
					let query_time = timer.elapsed();

					(results, query_time)
				},
				(None, Some(server_name), None, None) => {
					let timer = tokio::time::Instant::now();
					let results = services()
						.sending
						.db
						.queued_requests(&Destination::Normal(server_name.into()));
					let query_time = timer.elapsed();

					(results, query_time)
				},
				(None, None, Some(user_id), Some(push_key)) => {
					if push_key.is_empty() {
						return Ok(RoomMessageEventContent::text_plain(
							"An appservice ID, server name, or a user ID with push key must be specified via \
							 arguments. See --help for more details.",
						));
					}

					let timer = tokio::time::Instant::now();
					let results = services()
						.sending
						.db
						.queued_requests(&Destination::Push(user_id.into(), push_key));
					let query_time = timer.elapsed();

					(results, query_time)
				},
				(Some(_), Some(_), Some(_), Some(_)) => {
					return Ok(RoomMessageEventContent::text_plain(
						"An appservice ID, server name, or a user ID with push key must be specified via arguments. \
						 Not all of them See --help for more details.",
					));
				},
				_ => {
					return Ok(RoomMessageEventContent::text_plain(
						"An appservice ID, server name, or a user ID with push key must be specified via arguments. \
						 See --help for more details.",
					));
				},
			};

			let queued_requests = results.collect::<Result<Vec<(_, _)>>>();

			Ok(RoomMessageEventContent::text_html(
				format!("Query completed in {query_time:?}:\n\n```\n{:?}```", queued_requests),
				format!(
					"<p>Query completed in {query_time:?}:</p>\n<pre><code>{:?}\n</code></pre>",
					queued_requests
				),
			))
		},
		Sending::ActiveRequestsFor {
			appservice_id,
			server_name,
			user_id,
			push_key,
		} => {
			if appservice_id.is_none() && server_name.is_none() && user_id.is_none() && push_key.is_none() {
				return Ok(RoomMessageEventContent::text_plain(
					"An appservice ID, server name, or a user ID with push key must be specified via arguments. See \
					 --help for more details.",
				));
			}

			let (results, query_time) = match (appservice_id, server_name, user_id, push_key) {
				(Some(appservice_id), None, None, None) => {
					if appservice_id.is_empty() {
						return Ok(RoomMessageEventContent::text_plain(
							"An appservice ID, server name, or a user ID with push key must be specified via \
							 arguments. See --help for more details.",
						));
					}

					let timer = tokio::time::Instant::now();
					let results = services()
						.sending
						.db
						.active_requests_for(&Destination::Appservice(appservice_id));
					let query_time = timer.elapsed();

					(results, query_time)
				},
				(None, Some(server_name), None, None) => {
					let timer = tokio::time::Instant::now();
					let results = services()
						.sending
						.db
						.active_requests_for(&Destination::Normal(server_name.into()));
					let query_time = timer.elapsed();

					(results, query_time)
				},
				(None, None, Some(user_id), Some(push_key)) => {
					if push_key.is_empty() {
						return Ok(RoomMessageEventContent::text_plain(
							"An appservice ID, server name, or a user ID with push key must be specified via \
							 arguments. See --help for more details.",
						));
					}

					let timer = tokio::time::Instant::now();
					let results = services()
						.sending
						.db
						.active_requests_for(&Destination::Push(user_id.into(), push_key));
					let query_time = timer.elapsed();

					(results, query_time)
				},
				(Some(_), Some(_), Some(_), Some(_)) => {
					return Ok(RoomMessageEventContent::text_plain(
						"An appservice ID, server name, or a user ID with push key must be specified via arguments. \
						 Not all of them See --help for more details.",
					));
				},
				_ => {
					return Ok(RoomMessageEventContent::text_plain(
						"An appservice ID, server name, or a user ID with push key must be specified via arguments. \
						 See --help for more details.",
					));
				},
			};

			let active_requests = results.collect::<Result<Vec<(_, _)>>>();

			Ok(RoomMessageEventContent::text_html(
				format!("Query completed in {query_time:?}:\n\n```\n{:?}```", active_requests),
				format!(
					"<p>Query completed in {query_time:?}:</p>\n<pre><code>{:?}\n</code></pre>",
					active_requests
				),
			))
		},
		Sending::GetLatestEduCount {
			server_name,
		} => {
			let timer = tokio::time::Instant::now();
			let results = services().sending.db.get_latest_educount(&server_name);
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
