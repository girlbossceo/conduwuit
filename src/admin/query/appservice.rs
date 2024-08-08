use clap::Subcommand;
use conduit::Result;
use ruma::events::room::message::RoomMessageEventContent;

use crate::Command;

#[derive(Debug, Subcommand)]
/// All the getters and iterators from src/database/key_value/appservice.rs
pub(crate) enum AppserviceCommand {
	/// - Gets the appservice registration info/details from the ID as a string
	GetRegistration {
		/// Appservice registration ID
		appservice_id: Box<str>,
	},

	/// - Gets all appservice registrations with their ID and registration info
	All,
}

/// All the getters and iterators from src/database/key_value/appservice.rs
pub(super) async fn process(subcommand: AppserviceCommand, context: &Command<'_>) -> Result<RoomMessageEventContent> {
	let services = context.services;

	match subcommand {
		AppserviceCommand::GetRegistration {
			appservice_id,
		} => {
			let timer = tokio::time::Instant::now();
			let results = services
				.appservice
				.db
				.get_registration(appservice_id.as_ref())
				.await;

			let query_time = timer.elapsed();

			Ok(RoomMessageEventContent::notice_markdown(format!(
				"Query completed in {query_time:?}:\n\n```rs\n{results:#?}\n```"
			)))
		},
		AppserviceCommand::All => {
			let timer = tokio::time::Instant::now();
			let results = services.appservice.all().await;
			let query_time = timer.elapsed();

			Ok(RoomMessageEventContent::notice_markdown(format!(
				"Query completed in {query_time:?}:\n\n```rs\n{results:#?}\n```"
			)))
		},
	}
}
