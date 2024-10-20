use clap::Subcommand;
use conduit::Result;
use ruma::{events::room::message::RoomMessageEventContent, UserId};

use crate::Command;

#[derive(Debug, Subcommand)]
pub(crate) enum PusherCommand {
	/// - Returns all the pushers for the user.
	GetPushers {
		/// Full user ID
		user_id: Box<UserId>,
	},
}

pub(super) async fn process(subcommand: PusherCommand, context: &Command<'_>) -> Result<RoomMessageEventContent> {
	let services = context.services;

	match subcommand {
		PusherCommand::GetPushers {
			user_id,
		} => {
			let timer = tokio::time::Instant::now();
			let results = services.pusher.get_pushers(&user_id).await;
			let query_time = timer.elapsed();

			Ok(RoomMessageEventContent::notice_markdown(format!(
				"Query completed in {query_time:?}:\n\n```rs\n{results:#?}\n```"
			)))
		},
	}
}
