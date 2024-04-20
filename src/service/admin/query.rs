use clap::Subcommand;
use ruma::{
	events::{room::message::RoomMessageEventContent, RoomAccountDataEventType},
	RoomId, UserId,
};

use crate::{services, Result};

#[cfg_attr(test, derive(Debug))]
#[derive(Subcommand)]
/// Query tables from database
pub(crate) enum QueryCommand {
	/// - account_data.rs iterators and getters
	#[command(subcommand)]
	AccountData(AccountData),

	/// - appservice.rs iterators and getters
	#[command(subcommand)]
	Appservice(Appservice),

	/// - presence.rs iterators and getters
	#[command(subcommand)]
	Presence(Presence),
}

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

#[cfg_attr(test, derive(Debug))]
#[derive(Subcommand)]
/// All the getters and iterators from src/database/key_value/appservice.rs via
/// services()
pub(crate) enum Appservice {
	/// - Gets the appservice registration info/details from the ID as a string
	GetRegistration {
		/// Appservice registration ID
		appservice_id: Box<str>,
	},
}

#[cfg_attr(test, derive(Debug))]
#[derive(Subcommand)]
/// All the getters and iterators from src/database/key_value/presence.rs via
/// services()
pub(crate) enum Presence {
	/// - Returns the latest presence event for the given user.
	GetPresence {
		/// Full user ID
		user_id: Box<UserId>,
	},

	/// - Returns the most recent presence updates that happened after the event
	///   with id `since`.
	PresenceSince {
		/// UNIX timestamp since (u64)
		since: u64,
	},
}

/// Processes admin command
#[allow(non_snake_case)]
pub(crate) async fn process(command: QueryCommand, _body: Vec<&str>) -> Result<RoomMessageEventContent> {
	match command {
		QueryCommand::AccountData(AccountData) => account_data(AccountData).await,
		QueryCommand::Appservice(Appservice) => appservice(Appservice).await,
		QueryCommand::Presence(Presence) => presence(Presence).await,
	}
}

/// All the getters and iterators in key_value/account_data.rs via services()
async fn account_data(subcommand: AccountData) -> Result<RoomMessageEventContent> {
	match subcommand {
		AccountData::ChangesSince {
			user_id,
			since,
			room_id,
		} => {
			let timer = tokio::time::Instant::now();
			let results = services()
				.account_data
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

/// All the getters and iterators in key_value/appservice.rs via services()
async fn appservice(subcommand: Appservice) -> Result<RoomMessageEventContent> {
	match subcommand {
		Appservice::GetRegistration {
			appservice_id,
		} => {
			let timer = tokio::time::Instant::now();
			let results = services()
				.appservice
				.get_registration(appservice_id.as_ref())
				.await;
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

/// All the getters and iterators in key_value/appservice.rs via services()
async fn presence(subcommand: Presence) -> Result<RoomMessageEventContent> {
	match subcommand {
		Presence::GetPresence {
			user_id,
		} => {
			let timer = tokio::time::Instant::now();
			let results = services().presence.get_presence(&user_id)?;
			let query_time = timer.elapsed();

			Ok(RoomMessageEventContent::text_html(
				format!("Query completed in {query_time:?}:\n\n```\n{:?}```", results),
				format!(
					"<p>Query completed in {query_time:?}:</p>\n<pre><code>{:?}\n</code></pre>",
					results
				),
			))
		},
		Presence::PresenceSince {
			since,
		} => {
			let timer = tokio::time::Instant::now();
			let results = services().presence.presence_since(since);
			let query_time = timer.elapsed();

			let presence_since: Vec<(_, _, _)> = results.collect();

			Ok(RoomMessageEventContent::text_html(
				format!("Query completed in {query_time:?}:\n\n```\n{:?}```", presence_since),
				format!(
					"<p>Query completed in {query_time:?}:</p>\n<pre><code>{:?}\n</code></pre>",
					presence_since
				),
			))
		},
	}
}
