use clap::Subcommand;
use ruma::events::room::message::RoomMessageEventContent;

use super::{
	account_data::{account_data, AccountData},
	appservice::{appservice, Appservice},
	globals::{globals, Globals},
	presence::{presence, Presence},
	room_alias::{room_alias, RoomAlias},
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
}

/// Processes admin query commands
#[allow(non_snake_case)]
pub(crate) async fn process(command: QueryCommand, _body: Vec<&str>) -> Result<RoomMessageEventContent> {
	match command {
		QueryCommand::AccountData(AccountData) => account_data(AccountData).await,
		QueryCommand::Appservice(Appservice) => appservice(Appservice).await,
		QueryCommand::Presence(Presence) => presence(Presence).await,
		QueryCommand::RoomAlias(RoomAlias) => room_alias(RoomAlias).await,
		QueryCommand::Globals(Globals) => globals(Globals).await,
	}
}
