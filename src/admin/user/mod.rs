pub(crate) mod user_commands;

use clap::Subcommand;
use ruma::{events::room::message::RoomMessageEventContent, RoomId};
use user_commands::{delete_room_tag, get_room_tags, put_room_tag};

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
	/// Can be overridden with --leave-rooms OR the --force flag.
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
		/// Also deactivate admin accounts and will assume leave all rooms too
		force: bool,
	},

	/// - List local users in the database
	List,

	/// - Lists all the rooms (local and remote) that the specified user is
	///   joined in
	ListJoinedRooms {
		user_id: String,
	},

	/// - Puts a room tag for the specified user and room ID.
	///
	/// This is primarily useful if you'd like to set your admin room
	/// to the special "System Alerts" section in Element as a way to
	/// permanently see your admin room without it being buried away in your
	/// favourites or rooms. To do this, you would pass your user, your admin
	/// room's internal ID, and the tag name `m.server_notice`.
	PutRoomTag {
		user_id: String,
		room_id: Box<RoomId>,
		tag: String,
	},

	/// - Deletes the room tag for the specified user and room ID
	DeleteRoomTag {
		user_id: String,
		room_id: Box<RoomId>,
		tag: String,
	},

	/// - Gets all the room tags for the specified user and room ID
	GetRoomTags {
		user_id: String,
		room_id: Box<RoomId>,
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
		UserCommand::PutRoomTag {
			user_id,
			room_id,
			tag,
		} => put_room_tag(body, user_id, room_id, tag).await?,
		UserCommand::DeleteRoomTag {
			user_id,
			room_id,
			tag,
		} => delete_room_tag(body, user_id, room_id, tag).await?,
		UserCommand::GetRoomTags {
			user_id,
			room_id,
		} => get_room_tags(body, user_id, room_id).await?,
	})
}
