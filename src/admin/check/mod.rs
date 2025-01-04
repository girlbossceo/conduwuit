mod commands;

use clap::Subcommand;
use conduwuit::Result;

use crate::admin_command_dispatch;

#[admin_command_dispatch]
#[derive(Debug, Subcommand)]
pub(super) enum CheckCommand {
	CheckAllUsers,
}
