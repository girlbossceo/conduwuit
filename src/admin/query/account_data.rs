use clap::Subcommand;
use conduit::Result;
use ruma::{
	events::{room::message::RoomMessageEventContent, RoomAccountDataEventType},
	RoomId, UserId,
};

use crate::Command;

#[derive(Debug, Subcommand)]
/// All the getters and iterators from src/database/key_value/account_data.rs
pub(crate) enum AccountDataCommand {
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

/// All the getters and iterators from src/database/key_value/account_data.rs
pub(super) async fn process(subcommand: AccountDataCommand, context: &Command<'_>) -> Result<RoomMessageEventContent> {
	let services = context.services;

	match subcommand {
		AccountDataCommand::ChangesSince {
			user_id,
			since,
			room_id,
		} => {
			let timer = tokio::time::Instant::now();
			let results = services
				.account_data
				.changes_since(room_id.as_deref(), &user_id, since)
				.await?;
			let query_time = timer.elapsed();

			Ok(RoomMessageEventContent::notice_markdown(format!(
				"Query completed in {query_time:?}:\n\n```rs\n{results:#?}\n```"
			)))
		},
		AccountDataCommand::Get {
			user_id,
			kind,
			room_id,
		} => {
			let timer = tokio::time::Instant::now();
			let results = services
				.account_data
				.get(room_id.as_deref(), &user_id, kind)
				.await;
			let query_time = timer.elapsed();

			Ok(RoomMessageEventContent::notice_markdown(format!(
				"Query completed in {query_time:?}:\n\n```rs\n{results:#?}\n```"
			)))
		},
	}
}
