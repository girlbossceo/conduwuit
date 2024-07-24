use clap::Parser;
use conduit::Result;
use ruma::events::room::message::RoomMessageEventContent;

use crate::{
	appservice, appservice::AppserviceCommand, check, check::CheckCommand, debug, debug::DebugCommand, federation,
	federation::FederationCommand, media, media::MediaCommand, query, query::QueryCommand, room, room::RoomCommand,
	server, server::ServerCommand, user, user::UserCommand,
};

#[derive(Debug, Parser)]
#[command(name = "admin", version = env!("CARGO_PKG_VERSION"))]
pub(crate) enum AdminCommand {
	#[command(subcommand)]
	/// - Commands for managing appservices
	Appservices(AppserviceCommand),

	#[command(subcommand)]
	/// - Commands for managing local users
	Users(UserCommand),

	#[command(subcommand)]
	/// - Commands for managing rooms
	Rooms(RoomCommand),

	#[command(subcommand)]
	/// - Commands for managing federation
	Federation(FederationCommand),

	#[command(subcommand)]
	/// - Commands for managing the server
	Server(ServerCommand),

	#[command(subcommand)]
	/// - Commands for managing media
	Media(MediaCommand),

	#[command(subcommand)]
	/// - Commands for checking integrity
	Check(CheckCommand),

	#[command(subcommand)]
	/// - Commands for debugging things
	Debug(DebugCommand),

	#[command(subcommand)]
	/// - Low-level queries for database getters and iterators
	Query(QueryCommand),
}

#[tracing::instrument(skip_all, name = "command")]
pub(crate) async fn process(command: AdminCommand, body: Vec<&str>) -> Result<RoomMessageEventContent> {
	let reply_message_content = match command {
		AdminCommand::Appservices(command) => appservice::process(command, body).await?,
		AdminCommand::Media(command) => media::process(command, body).await?,
		AdminCommand::Users(command) => user::process(command, body).await?,
		AdminCommand::Rooms(command) => room::process(command, body).await?,
		AdminCommand::Federation(command) => federation::process(command, body).await?,
		AdminCommand::Server(command) => server::process(command, body).await?,
		AdminCommand::Debug(command) => debug::process(command, body).await?,
		AdminCommand::Query(command) => query::process(command, body).await?,
		AdminCommand::Check(command) => check::process(command, body).await?,
	};

	Ok(reply_message_content)
}
