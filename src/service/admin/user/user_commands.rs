use std::{fmt::Write as _, sync::Arc};

use ruma::{events::room::message::RoomMessageEventContent, OwnedRoomId, UserId};
use tracing::{error, info, warn};

use crate::{
	api::client_server::{join_room_by_id_helper, leave_all_rooms, AUTO_GEN_PASSWORD_LENGTH},
	service::admin::{escape_html, get_room_info},
	services,
	utils::{self, user_id::user_is_local},
	Result,
};

pub(crate) async fn list(_body: Vec<&str>) -> Result<RoomMessageEventContent> {
	match services().users.list_local_users() {
		Ok(users) => {
			let mut msg = format!("Found {} local user account(s):\n", users.len());
			msg += &users.join("\n");
			Ok(RoomMessageEventContent::text_plain(&msg))
		},
		Err(e) => Ok(RoomMessageEventContent::text_plain(e.to_string())),
	}
}

pub(crate) async fn create(
	_body: Vec<&str>, username: String, password: Option<String>,
) -> Result<RoomMessageEventContent> {
	let password = password.unwrap_or_else(|| utils::random_string(AUTO_GEN_PASSWORD_LENGTH));

	// Validate user id
	let user_id =
		match UserId::parse_with_server_name(username.as_str().to_lowercase(), services().globals.server_name()) {
			Ok(id) => id,
			Err(e) => {
				return Ok(RoomMessageEventContent::text_plain(format!(
					"The supplied username is not a valid username: {e}"
				)))
			},
		};

	if !user_is_local(&user_id) {
		return Ok(RoomMessageEventContent::text_plain(format!(
			"User {user_id} does not belong to our server."
		)));
	}

	if user_id.is_historical() {
		return Ok(RoomMessageEventContent::text_plain(format!(
			"Userid {user_id} is not allowed due to historical"
		)));
	}

	if services().users.exists(&user_id)? {
		return Ok(RoomMessageEventContent::text_plain(format!("Userid {user_id} already exists")));
	}
	// Create user
	services().users.create(&user_id, Some(password.as_str()))?;

	// Default to pretty displayname
	let mut displayname = user_id.localpart().to_owned();

	// If `new_user_displayname_suffix` is set, registration will push whatever
	// content is set to the user's display name with a space before it
	if !services()
		.globals
		.config
		.new_user_displayname_suffix
		.is_empty()
	{
		_ = write!(displayname, " {}", services().globals.config.new_user_displayname_suffix);
	}

	services()
		.users
		.set_displayname(&user_id, Some(displayname))
		.await?;

	// Initial account data
	services().account_data.update(
		None,
		&user_id,
		ruma::events::GlobalAccountDataEventType::PushRules
			.to_string()
			.into(),
		&serde_json::to_value(ruma::events::push_rules::PushRulesEvent {
			content: ruma::events::push_rules::PushRulesEventContent {
				global: ruma::push::Ruleset::server_default(&user_id),
			},
		})
		.expect("to json value always works"),
	)?;

	if !services().globals.config.auto_join_rooms.is_empty() {
		for room in &services().globals.config.auto_join_rooms {
			if !services()
				.rooms
				.state_cache
				.server_in_room(services().globals.server_name(), room)?
			{
				warn!("Skipping room {room} to automatically join as we have never joined before.");
				continue;
			}

			if let Some(room_id_server_name) = room.server_name() {
				match join_room_by_id_helper(
					Some(&user_id),
					room,
					Some("Automatically joining this room upon registration".to_owned()),
					&[room_id_server_name.to_owned(), services().globals.server_name().to_owned()],
					None,
				)
				.await
				{
					Ok(_) => {
						info!("Automatically joined room {room} for user {user_id}");
					},
					Err(e) => {
						// don't return this error so we don't fail registrations
						error!("Failed to automatically join room {room} for user {user_id}: {e}");
					},
				};
			}
		}
	}

	// we dont add a device since we're not the user, just the creator

	// Inhibit login does not work for guests
	Ok(RoomMessageEventContent::text_plain(format!(
		"Created user with user_id: {user_id} and password: `{password}`"
	)))
}

pub(crate) async fn deactivate(
	_body: Vec<&str>, leave_rooms: bool, user_id: String,
) -> Result<RoomMessageEventContent> {
	// Validate user id
	let user_id =
		match UserId::parse_with_server_name(user_id.as_str().to_lowercase(), services().globals.server_name()) {
			Ok(id) => Arc::<UserId>::from(id),
			Err(e) => {
				return Ok(RoomMessageEventContent::text_plain(format!(
					"The supplied username is not a valid username: {e}"
				)))
			},
		};

	// check if user belongs to our server
	if user_id.server_name() != services().globals.server_name() {
		return Ok(RoomMessageEventContent::text_plain(format!(
			"User {user_id} does not belong to our server."
		)));
	}

	// don't deactivate the conduit service account
	if user_id
		== UserId::parse_with_server_name("conduit", services().globals.server_name()).expect("conduit user exists")
	{
		return Ok(RoomMessageEventContent::text_plain(
			"Not allowed to deactivate the Conduit service account.",
		));
	}

	if services().users.exists(&user_id)? {
		RoomMessageEventContent::text_plain(format!("Making {user_id} leave all rooms before deactivation..."));

		services().users.deactivate_account(&user_id)?;

		if leave_rooms {
			leave_all_rooms(&user_id).await?;
		}

		Ok(RoomMessageEventContent::text_plain(format!(
			"User {user_id} has been deactivated"
		)))
	} else {
		Ok(RoomMessageEventContent::text_plain(format!(
			"User {user_id} doesn't exist on this server"
		)))
	}
}

pub(crate) async fn reset_password(_body: Vec<&str>, username: String) -> Result<RoomMessageEventContent> {
	// Validate user id
	let user_id =
		match UserId::parse_with_server_name(username.as_str().to_lowercase(), services().globals.server_name()) {
			Ok(id) => Arc::<UserId>::from(id),
			Err(e) => {
				return Ok(RoomMessageEventContent::text_plain(format!(
					"The supplied username is not a valid username: {e}"
				)))
			},
		};

	// check if user belongs to our server
	if user_id.server_name() != services().globals.server_name() {
		return Ok(RoomMessageEventContent::text_plain(format!(
			"User {user_id} does not belong to our server."
		)));
	}

	// Check if the specified user is valid
	if !services().users.exists(&user_id)?
		|| user_id
			== UserId::parse_with_server_name("conduit", services().globals.server_name()).expect("conduit user exists")
	{
		return Ok(RoomMessageEventContent::text_plain("The specified user does not exist!"));
	}

	let new_password = utils::random_string(AUTO_GEN_PASSWORD_LENGTH);

	match services()
		.users
		.set_password(&user_id, Some(new_password.as_str()))
	{
		Ok(()) => Ok(RoomMessageEventContent::text_plain(format!(
			"Successfully reset the password for user {user_id}: `{new_password}`"
		))),
		Err(e) => Ok(RoomMessageEventContent::text_plain(format!(
			"Couldn't reset the password for user {user_id}: {e}"
		))),
	}
}

pub(crate) async fn deactivate_all(body: Vec<&str>, leave_rooms: bool, force: bool) -> Result<RoomMessageEventContent> {
	if body.len() > 2 && body[0].trim().starts_with("```") && body.last().unwrap().trim() == "```" {
		let usernames = body.clone().drain(1..body.len() - 1).collect::<Vec<_>>();

		let mut user_ids: Vec<&UserId> = Vec::new();

		for &username in &usernames {
			match <&UserId>::try_from(username) {
				Ok(user_id) => user_ids.push(user_id),
				Err(e) => {
					return Ok(RoomMessageEventContent::text_plain(format!(
						"{username} is not a valid username: {e}"
					)))
				},
			}
		}

		let mut deactivation_count = 0;
		let mut admins = Vec::new();

		if !force {
			user_ids.retain(|&user_id| match services().users.is_admin(user_id) {
				Ok(is_admin) => {
					if is_admin {
						admins.push(user_id.localpart());
						false
					} else {
						true
					}
				},
				Err(_) => false,
			});
		}

		for &user_id in &user_ids {
			// check if user belongs to our server and skips over non-local users
			if user_id.server_name() != services().globals.server_name() {
				continue;
			}

			// don't deactivate the conduit service account
			if user_id
				== UserId::parse_with_server_name("conduit", services().globals.server_name())
					.expect("conduit user exists")
			{
				continue;
			}

			// user does not exist on our server
			if !services().users.exists(user_id)? {
				continue;
			}

			if services().users.deactivate_account(user_id).is_ok() {
				deactivation_count += 1;
			}
		}

		if leave_rooms {
			for &user_id in &user_ids {
				_ = leave_all_rooms(user_id).await;
			}
		}

		if admins.is_empty() {
			Ok(RoomMessageEventContent::text_plain(format!(
				"Deactivated {deactivation_count} accounts."
			)))
		} else {
			Ok(RoomMessageEventContent::text_plain(format!(
				"Deactivated {} accounts.\nSkipped admin accounts: {:?}. Use --force to deactivate admin accounts",
				deactivation_count,
				admins.join(", ")
			)))
		}
	} else {
		Ok(RoomMessageEventContent::text_plain(
			"Expected code block in command body. Add --help for details.",
		))
	}
}

pub(crate) async fn list_joined_rooms(_body: Vec<&str>, user_id: String) -> Result<RoomMessageEventContent> {
	// Validate user id
	let user_id =
		match UserId::parse_with_server_name(user_id.as_str().to_lowercase(), services().globals.server_name()) {
			Ok(id) => Arc::<UserId>::from(id),
			Err(e) => {
				return Ok(RoomMessageEventContent::text_plain(format!(
					"The supplied username is not a valid username: {e}"
				)))
			},
		};

	if user_id.server_name() != services().globals.server_name() {
		return Ok(RoomMessageEventContent::text_plain("User does not belong to our server."));
	}

	if !services().users.exists(&user_id)? {
		return Ok(RoomMessageEventContent::text_plain("User does not exist on this server."));
	}

	let mut rooms: Vec<(OwnedRoomId, u64, String)> = services()
		.rooms
		.state_cache
		.rooms_joined(&user_id)
		.filter_map(Result::ok)
		.map(|room_id| get_room_info(&room_id))
		.collect();

	if rooms.is_empty() {
		return Ok(RoomMessageEventContent::text_plain("User is not in any rooms."));
	}

	rooms.sort_by_key(|r| r.1);
	rooms.reverse();

	let output_plain = format!(
		"Rooms {user_id} Joined:\n{}",
		rooms
			.iter()
			.map(|(id, members, name)| format!("{id}\tMembers: {members}\tName: {name}"))
			.collect::<Vec<_>>()
			.join("\n")
	);
	let output_html = format!(
		"<table><caption>Rooms {user_id} \
		 Joined</caption>\n<tr><th>id</th>\t<th>members</th>\t<th>name</th></tr>\n{}</table>",
		rooms
			.iter()
			.fold(String::new(), |mut output, (id, members, name)| {
				writeln!(
					output,
					"<tr><td>{}</td>\t<td>{}</td>\t<td>{}</td></tr>",
					escape_html(id.as_ref()),
					members,
					escape_html(name)
				)
				.unwrap();
				output
			})
	);
	Ok(RoomMessageEventContent::text_html(output_plain, output_html))
}
