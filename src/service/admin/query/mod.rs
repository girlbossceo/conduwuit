pub(crate) mod account_data;
pub(crate) mod appservice;
pub(crate) mod globals;
pub(crate) mod presence;
pub(crate) mod room_alias;
pub(crate) mod sending;
pub(crate) mod users;

use clap::Subcommand;
use ruma::{
	events::{room::message::RoomMessageEventContent, RoomAccountDataEventType},
	RoomAliasId, RoomId, ServerName, UserId,
};

use self::{
	account_data::account_data, appservice::appservice, globals::globals, presence::presence, room_alias::room_alias,
	sending::sending, users::users,
};
use crate::Result;

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

	/// - rooms/alias.rs iterators and getters
	#[command(subcommand)]
	RoomAlias(RoomAlias),

	/// - globals.rs iterators and getters
	#[command(subcommand)]
	Globals(Globals),

	/// - sending.rs iterators and getters
	#[command(subcommand)]
	Sending(Sending),

	/// - users.rs iterators and getters
	#[command(subcommand)]
	Users(Users),
}

#[cfg_attr(test, derive(Debug))]
#[derive(Subcommand)]
/// All the getters and iterators from src/database/key_value/account_data.rs
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
/// All the getters and iterators from src/database/key_value/appservice.rs
pub(crate) enum Appservice {
	/// - Gets the appservice registration info/details from the ID as a string
	GetRegistration {
		/// Appservice registration ID
		appservice_id: Box<str>,
	},

	/// - Gets all appservice registrations with their ID and registration info
	All,
}

#[cfg_attr(test, derive(Debug))]
#[derive(Subcommand)]
/// All the getters and iterators from src/database/key_value/presence.rs
pub(crate) enum Presence {
	/// - Returns the latest presence event for the given user.
	GetPresence {
		/// Full user ID
		user_id: Box<UserId>,
	},

	/// - Iterator of the most recent presence updates that happened after the
	///   event with id `since`.
	PresenceSince {
		/// UNIX timestamp since (u64)
		since: u64,
	},
}

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

#[cfg_attr(test, derive(Debug))]
#[derive(Subcommand)]
/// All the getters and iterators from src/database/key_value/globals.rs
pub(crate) enum Globals {
	DatabaseVersion,

	CurrentCount,

	LastCheckForUpdatesId,

	LoadKeypair,

	/// - This returns an empty `Ok(BTreeMap<..>)` when there are no keys found
	///   for the server.
	SigningKeysFor {
		origin: Box<ServerName>,
	},
}

#[cfg_attr(test, derive(Debug))]
#[derive(Subcommand)]
/// All the getters and iterators from src/database/key_value/sending.rs
pub(crate) enum Sending {
	/// - Queries database for all `servercurrentevent_data`
	ActiveRequests,

	/// - Queries database for `servercurrentevent_data` but for a specific
	///   destination
	///
	/// This command takes only *one* format of these arguments:
	///
	/// appservice_id
	/// server_name
	/// user_id AND push_key
	///
	/// See src/service/sending/mod.rs for the definition of the `Destination`
	/// enum
	ActiveRequestsFor {
		#[arg(short, long)]
		appservice_id: Option<String>,
		#[arg(short, long)]
		server_name: Option<Box<ServerName>>,
		#[arg(short, long)]
		user_id: Option<Box<UserId>>,
		#[arg(short, long)]
		push_key: Option<String>,
	},

	/// - Queries database for `servernameevent_data` which are the queued up
	///   requests that will eventually be sent
	///
	/// This command takes only *one* format of these arguments:
	///
	/// appservice_id
	/// server_name
	/// user_id AND push_key
	///
	/// See src/service/sending/mod.rs for the definition of the `Destination`
	/// enum
	QueuedRequests {
		#[arg(short, long)]
		appservice_id: Option<String>,
		#[arg(short, long)]
		server_name: Option<Box<ServerName>>,
		#[arg(short, long)]
		user_id: Option<Box<UserId>>,
		#[arg(short, long)]
		push_key: Option<String>,
	},

	GetLatestEduCount {
		server_name: Box<ServerName>,
	},
}

#[cfg_attr(test, derive(Debug))]
#[derive(Subcommand)]
/// All the getters and iterators from src/database/key_value/users.rs
pub(crate) enum Users {
	Iter,
}

/// Processes admin query commands
pub(crate) async fn process(command: QueryCommand, _body: Vec<&str>) -> Result<RoomMessageEventContent> {
	Ok(match command {
		QueryCommand::AccountData(command) => account_data(command).await?,
		QueryCommand::Appservice(command) => appservice(command).await?,
		QueryCommand::Presence(command) => presence(command).await?,
		QueryCommand::RoomAlias(command) => room_alias(command).await?,
		QueryCommand::Globals(command) => globals(command).await?,
		QueryCommand::Sending(command) => sending(command).await?,
		QueryCommand::Users(command) => users(command).await?,
	})
}
