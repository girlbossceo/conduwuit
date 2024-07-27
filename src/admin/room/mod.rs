mod alias;
mod commands;
mod directory;
mod info;
mod moderation;

use clap::Subcommand;
use conduit::Result;

use self::{
	alias::RoomAliasCommand, directory::RoomDirectoryCommand, info::RoomInfoCommand, moderation::RoomModerationCommand,
};
use crate::admin_command_dispatch;

#[admin_command_dispatch]
#[derive(Debug, Subcommand)]
pub(super) enum RoomCommand {
	/// - List all rooms the server knows about
	#[clap(alias = "list")]
	ListRooms {
		page: Option<usize>,

		/// Excludes rooms that we have federation disabled with
		#[arg(long)]
		exclude_disabled: bool,

		/// Excludes rooms that we have banned
		#[arg(long)]
		exclude_banned: bool,
	},

	#[command(subcommand)]
	/// - View information about a room we know about
	Info(RoomInfoCommand),

	#[command(subcommand)]
	/// - Manage moderation of remote or local rooms
	Moderation(RoomModerationCommand),

	#[command(subcommand)]
	/// - Manage rooms' aliases
	Alias(RoomAliasCommand),

	#[command(subcommand)]
	/// - Manage the room directory
	Directory(RoomDirectoryCommand),
}
