use ruma::events::room::message::RoomMessageEventContent;

use super::Appservice;
use crate::{services, Result};

/// All the getters and iterators from src/database/key_value/appservice.rs
pub(super) async fn appservice(subcommand: Appservice) -> Result<RoomMessageEventContent> {
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

			Ok(RoomMessageEventContent::notice_markdown(format!(
				"Query completed in {query_time:?}:\n\n```rs\n{results:#?}\n```"
			)))
		},
		Appservice::All => {
			let timer = tokio::time::Instant::now();
			let results = services().appservice.db.all();
			let query_time = timer.elapsed();

			Ok(RoomMessageEventContent::notice_markdown(format!(
				"Query completed in {query_time:?}:\n\n```rs\n{results:#?}\n```"
			)))
		},
	}
}
