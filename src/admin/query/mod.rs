mod account_data;
mod appservice;
mod globals;
mod presence;
mod pusher;
mod resolver;
mod room_alias;
mod room_state_cache;
mod sending;
mod users;

use clap::Subcommand;
use conduwuit::Result;

use self::{
	account_data::AccountDataCommand, appservice::AppserviceCommand, globals::GlobalsCommand,
	presence::PresenceCommand, pusher::PusherCommand, resolver::ResolverCommand,
	room_alias::RoomAliasCommand, room_state_cache::RoomStateCacheCommand,
	sending::SendingCommand, users::UsersCommand,
};
use crate::admin_command_dispatch;

#[admin_command_dispatch]
#[derive(Debug, Subcommand)]
/// Query tables from database
pub(super) enum QueryCommand {
	/// - account_data.rs iterators and getters
	#[command(subcommand)]
	AccountData(AccountDataCommand),

	/// - appservice.rs iterators and getters
	#[command(subcommand)]
	Appservice(AppserviceCommand),

	/// - presence.rs iterators and getters
	#[command(subcommand)]
	Presence(PresenceCommand),

	/// - rooms/alias.rs iterators and getters
	#[command(subcommand)]
	RoomAlias(RoomAliasCommand),

	/// - rooms/state_cache iterators and getters
	#[command(subcommand)]
	RoomStateCache(RoomStateCacheCommand),

	/// - globals.rs iterators and getters
	#[command(subcommand)]
	Globals(GlobalsCommand),

	/// - sending.rs iterators and getters
	#[command(subcommand)]
	Sending(SendingCommand),

	/// - users.rs iterators and getters
	#[command(subcommand)]
	Users(UsersCommand),

	/// - resolver service
	#[command(subcommand)]
	Resolver(ResolverCommand),

	/// - pusher service
	#[command(subcommand)]
	Pusher(PusherCommand),
}
