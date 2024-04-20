#[allow(clippy::module_inception)]
pub(crate) mod user;

use clap::Subcommand;
use ruma::UserId;

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
		user_id: Box<UserId>,
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
		user_id: Box<UserId>,
	},
}
