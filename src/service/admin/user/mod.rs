pub(crate) mod user_commands;

use clap::Subcommand;
use ruma::events::room::message::RoomMessageEventContent;

use self::user_commands::{create, deactivate, deactivate_all, list, list_joined_rooms, reset_password};
use crate::Result;

#[cfg_attr(test, derive(Debug))]
#[derive(Subcommand)]
pub(crate) enum UserCommand {
	/// - Create a new user
	Create {
		/// Username of the new user
		username: String,
		/// Password of the new user, if unspecified one is generated
		password: Option<String>,
	},

	/// - Reset user password
	ResetPassword {
		/// Username of the user for whom the password should be reset
		username: String,
	},

	/// - Deactivate a user
	///
	/// User will not be removed from all rooms by default.
	/// Use --leave-rooms to force the user to leave all rooms
	Deactivate {
		#[arg(short, long)]
		leave_rooms: bool,
		user_id: String,
	},

	/// - Deactivate a list of users
	///
	/// Recommended to use in conjunction with list-local-users.
	///
	/// Users will not be removed from joined rooms by default.
	/// Can be overridden with --leave-rooms flag.
	/// Removing a mass amount of users from a room may cause a significant
	/// amount of leave events. The time to leave rooms may depend significantly
	/// on joined rooms and servers.
	///
	/// This command needs a newline separated list of users provided in a
	/// Markdown code block below the command.
	DeactivateAll {
		#[arg(short, long)]
		/// Remove users from their joined rooms
		leave_rooms: bool,
		#[arg(short, long)]
		/// Also deactivate admin accounts
		force: bool,
	},

	/// - List local users in the database
	List,

	/// - Lists all the rooms (local and remote) that the specified user is
	///   joined in
	ListJoinedRooms {
		user_id: String,
	},
}

pub(crate) async fn process(command: UserCommand, body: Vec<&str>) -> Result<RoomMessageEventContent> {
	Ok(match command {
		UserCommand::List => list(body).await?,
		UserCommand::Create {
			username,
			password,
		} => create(body, username, password).await?,
		UserCommand::Deactivate {
			leave_rooms,
			user_id,
		} => deactivate(body, leave_rooms, user_id).await?,
		UserCommand::ResetPassword {
			username,
		} => reset_password(body, username).await?,
		UserCommand::DeactivateAll {
			leave_rooms,
			force,
		} => deactivate_all(body, leave_rooms, force).await?,
		UserCommand::ListJoinedRooms {
			user_id,
		} => list_joined_rooms(body, user_id).await?,
	})
}
