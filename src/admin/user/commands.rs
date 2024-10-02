use std::{collections::BTreeMap, fmt::Write as _};

use api::client::{full_user_deactivate, join_room_by_id_helper, leave_room};
use conduit::{error, info, is_equal_to, utils, warn, PduBuilder, Result};
use conduit_api::client::{leave_all_rooms, update_avatar_url, update_displayname};
use futures::StreamExt;
use ruma::{
	events::{
		room::{
			message::RoomMessageEventContent,
			power_levels::{RoomPowerLevels, RoomPowerLevelsEventContent},
			redaction::RoomRedactionEventContent,
		},
		tag::{TagEvent, TagEventContent, TagInfo},
		RoomAccountDataEventType, StateEventType, TimelineEventType,
	},
	EventId, OwnedRoomId, OwnedRoomOrAliasId, OwnedUserId, RoomId,
};
use serde_json::value::to_raw_value;

use crate::{
	admin_command, get_room_info,
	utils::{parse_active_local_user_id, parse_local_user_id},
};

const AUTO_GEN_PASSWORD_LENGTH: usize = 25;

#[admin_command]
pub(super) async fn list_users(&self) -> Result<RoomMessageEventContent> {
	let users = self
		.services
		.users
		.list_local_users()
		.map(ToString::to_string)
		.collect::<Vec<_>>()
		.await;

	let mut plain_msg = format!("Found {} local user account(s):\n```\n", users.len());
	plain_msg += users.join("\n").as_str();
	plain_msg += "\n```";

	Ok(RoomMessageEventContent::notice_markdown(plain_msg))
}

#[admin_command]
pub(super) async fn create_user(&self, username: String, password: Option<String>) -> Result<RoomMessageEventContent> {
	// Validate user id
	let user_id = parse_local_user_id(self.services, &username)?;

	if self.services.users.exists(&user_id).await {
		return Ok(RoomMessageEventContent::text_plain(format!("Userid {user_id} already exists")));
	}

	if user_id.is_historical() {
		return Ok(RoomMessageEventContent::text_plain(format!(
			"User ID {user_id} does not conform to new Matrix identifier spec"
		)));
	}

	let password = password.unwrap_or_else(|| utils::random_string(AUTO_GEN_PASSWORD_LENGTH));

	// Create user
	self.services
		.users
		.create(&user_id, Some(password.as_str()))?;

	// Default to pretty displayname
	let mut displayname = user_id.localpart().to_owned();

	// If `new_user_displayname_suffix` is set, registration will push whatever
	// content is set to the user's display name with a space before it
	if !self
		.services
		.globals
		.config
		.new_user_displayname_suffix
		.is_empty()
	{
		write!(displayname, " {}", self.services.globals.config.new_user_displayname_suffix)
			.expect("should be able to write to string buffer");
	}

	self.services
		.users
		.set_displayname(&user_id, Some(displayname));

	// Initial account data
	self.services
		.account_data
		.update(
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
		)
		.await?;

	if !self.services.globals.config.auto_join_rooms.is_empty() {
		for room in &self.services.globals.config.auto_join_rooms {
			if !self
				.services
				.rooms
				.state_cache
				.server_in_room(self.services.globals.server_name(), room)
				.await
			{
				warn!("Skipping room {room} to automatically join as we have never joined before.");
				continue;
			}

			if let Some(room_id_server_name) = room.server_name() {
				match join_room_by_id_helper(
					self.services,
					&user_id,
					room,
					Some("Automatically joining this room upon registration".to_owned()),
					&[room_id_server_name.to_owned(), self.services.globals.server_name().to_owned()],
					None,
					&None,
				)
				.await
				{
					Ok(_response) => {
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

	// if this account creation is from the CLI / --execute, invite the first user
	// to admin room
	if let Ok(admin_room) = self.services.admin.get_admin_room().await {
		if self
			.services
			.rooms
			.state_cache
			.room_joined_count(&admin_room)
			.await
			.is_ok_and(is_equal_to!(1))
		{
			self.services.admin.make_user_admin(&user_id).await?;

			warn!("Granting {user_id} admin privileges as the first user");
		}
	}

	// Inhibit login does not work for guests
	Ok(RoomMessageEventContent::text_plain(format!(
		"Created user with user_id: {user_id} and password: `{password}`"
	)))
}

#[admin_command]
pub(super) async fn deactivate(&self, no_leave_rooms: bool, user_id: String) -> Result<RoomMessageEventContent> {
	// Validate user id
	let user_id = parse_local_user_id(self.services, &user_id)?;

	// don't deactivate the server service account
	if user_id == self.services.globals.server_user {
		return Ok(RoomMessageEventContent::text_plain(
			"Not allowed to deactivate the server service account.",
		));
	}

	self.services.users.deactivate_account(&user_id).await?;

	if !no_leave_rooms {
		self.services
			.admin
			.send_message(RoomMessageEventContent::text_plain(format!(
				"Making {user_id} leave all rooms after deactivation..."
			)))
			.await
			.ok();

		let all_joined_rooms: Vec<OwnedRoomId> = self
			.services
			.rooms
			.state_cache
			.rooms_joined(&user_id)
			.map(Into::into)
			.collect()
			.await;

		full_user_deactivate(self.services, &user_id, &all_joined_rooms).await?;
		update_displayname(self.services, &user_id, None, &all_joined_rooms).await?;
		update_avatar_url(self.services, &user_id, None, None, &all_joined_rooms).await?;
		leave_all_rooms(self.services, &user_id).await;
	}

	Ok(RoomMessageEventContent::text_plain(format!(
		"User {user_id} has been deactivated"
	)))
}

#[admin_command]
pub(super) async fn reset_password(&self, username: String) -> Result<RoomMessageEventContent> {
	let user_id = parse_local_user_id(self.services, &username)?;

	if user_id == self.services.globals.server_user {
		return Ok(RoomMessageEventContent::text_plain(
			"Not allowed to set the password for the server account. Please use the emergency password config option.",
		));
	}

	let new_password = utils::random_string(AUTO_GEN_PASSWORD_LENGTH);

	match self
		.services
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

#[admin_command]
pub(super) async fn deactivate_all(&self, no_leave_rooms: bool, force: bool) -> Result<RoomMessageEventContent> {
	if self.body.len() < 2 || !self.body[0].trim().starts_with("```") || self.body.last().unwrap_or(&"").trim() != "```"
	{
		return Ok(RoomMessageEventContent::text_plain(
			"Expected code block in command body. Add --help for details.",
		));
	}

	let usernames = self
		.body
		.to_vec()
		.drain(1..self.body.len().saturating_sub(1))
		.collect::<Vec<_>>();

	let mut user_ids: Vec<OwnedUserId> = Vec::with_capacity(usernames.len());
	let mut admins = Vec::new();

	for username in usernames {
		match parse_active_local_user_id(self.services, username).await {
			Ok(user_id) => {
				if self.services.users.is_admin(&user_id).await && !force {
					self.services
						.admin
						.send_message(RoomMessageEventContent::text_plain(format!(
							"{username} is an admin and --force is not set, skipping over"
						)))
						.await
						.ok();
					admins.push(username);
					continue;
				}

				// don't deactivate the server service account
				if user_id == self.services.globals.server_user {
					self.services
						.admin
						.send_message(RoomMessageEventContent::text_plain(format!(
							"{username} is the server service account, skipping over"
						)))
						.await
						.ok();
					continue;
				}

				user_ids.push(user_id);
			},
			Err(e) => {
				self.services
					.admin
					.send_message(RoomMessageEventContent::text_plain(format!(
						"{username} is not a valid username, skipping over: {e}"
					)))
					.await
					.ok();
				continue;
			},
		}
	}

	let mut deactivation_count: usize = 0;

	for user_id in user_ids {
		match self.services.users.deactivate_account(&user_id).await {
			Ok(()) => {
				deactivation_count = deactivation_count.saturating_add(1);
				if !no_leave_rooms {
					info!("Forcing user {user_id} to leave all rooms apart of deactivate-all");
					let all_joined_rooms: Vec<OwnedRoomId> = self
						.services
						.rooms
						.state_cache
						.rooms_joined(&user_id)
						.map(Into::into)
						.collect()
						.await;

					full_user_deactivate(self.services, &user_id, &all_joined_rooms).await?;
					update_displayname(self.services, &user_id, None, &all_joined_rooms)
						.await
						.ok();
					update_avatar_url(self.services, &user_id, None, None, &all_joined_rooms)
						.await
						.ok();
					leave_all_rooms(self.services, &user_id).await;
				}
			},
			Err(e) => {
				self.services
					.admin
					.send_message(RoomMessageEventContent::text_plain(format!("Failed deactivating user: {e}")))
					.await
					.ok();
			},
		}
	}

	if admins.is_empty() {
		Ok(RoomMessageEventContent::text_plain(format!(
			"Deactivated {deactivation_count} accounts."
		)))
	} else {
		Ok(RoomMessageEventContent::text_plain(format!(
			"Deactivated {deactivation_count} accounts.\nSkipped admin accounts: {}. Use --force to deactivate admin \
			 accounts",
			admins.join(", ")
		)))
	}
}

#[admin_command]
pub(super) async fn list_joined_rooms(&self, user_id: String) -> Result<RoomMessageEventContent> {
	// Validate user id
	let user_id = parse_local_user_id(self.services, &user_id)?;

	let mut rooms: Vec<(OwnedRoomId, u64, String)> = self
		.services
		.rooms
		.state_cache
		.rooms_joined(&user_id)
		.then(|room_id| get_room_info(self.services, room_id))
		.collect()
		.await;

	if rooms.is_empty() {
		return Ok(RoomMessageEventContent::text_plain("User is not in any rooms."));
	}

	rooms.sort_by_key(|r| r.1);
	rooms.reverse();

	let output_plain = format!(
		"Rooms {user_id} Joined ({}):\n```\n{}\n```",
		rooms.len(),
		rooms
			.iter()
			.map(|(id, members, name)| format!("{id}\tMembers: {members}\tName: {name}"))
			.collect::<Vec<_>>()
			.join("\n")
	);

	Ok(RoomMessageEventContent::notice_markdown(output_plain))
}

#[admin_command]
pub(super) async fn force_join_room(
	&self, user_id: String, room_id: OwnedRoomOrAliasId,
) -> Result<RoomMessageEventContent> {
	let user_id = parse_local_user_id(self.services, &user_id)?;
	let room_id = self.services.rooms.alias.resolve(&room_id).await?;

	assert!(
		self.services.globals.user_is_local(&user_id),
		"Parsed user_id must be a local user"
	);
	join_room_by_id_helper(self.services, &user_id, &room_id, None, &[], None, &None).await?;

	Ok(RoomMessageEventContent::notice_markdown(format!(
		"{user_id} has been joined to {room_id}.",
	)))
}

#[admin_command]
pub(super) async fn force_leave_room(
	&self, user_id: String, room_id: OwnedRoomOrAliasId,
) -> Result<RoomMessageEventContent> {
	let user_id = parse_local_user_id(self.services, &user_id)?;
	let room_id = self.services.rooms.alias.resolve(&room_id).await?;

	assert!(
		self.services.globals.user_is_local(&user_id),
		"Parsed user_id must be a local user"
	);
	leave_room(self.services, &user_id, &room_id, None).await?;

	Ok(RoomMessageEventContent::notice_markdown(format!(
		"{user_id} has left {room_id}.",
	)))
}

#[admin_command]
pub(super) async fn force_demote(
	&self, user_id: String, room_id: OwnedRoomOrAliasId,
) -> Result<RoomMessageEventContent> {
	let user_id = parse_local_user_id(self.services, &user_id)?;
	let room_id = self.services.rooms.alias.resolve(&room_id).await?;

	assert!(
		self.services.globals.user_is_local(&user_id),
		"Parsed user_id must be a local user"
	);

	let state_lock = self.services.rooms.state.mutex.lock(&room_id).await;

	let room_power_levels = self
		.services
		.rooms
		.state_accessor
		.room_state_get_content::<RoomPowerLevelsEventContent>(&room_id, &StateEventType::RoomPowerLevels, "")
		.await
		.ok();

	let user_can_demote_self = room_power_levels
		.as_ref()
		.is_some_and(|power_levels_content| {
			RoomPowerLevels::from(power_levels_content.clone()).user_can_change_user_power_level(&user_id, &user_id)
		}) || self
		.services
		.rooms
		.state_accessor
		.room_state_get(&room_id, &StateEventType::RoomCreate, "")
		.await
		.is_ok_and(|event| event.sender == user_id);

	if !user_can_demote_self {
		return Ok(RoomMessageEventContent::notice_markdown(
			"User is not allowed to modify their own power levels in the room.",
		));
	}

	let mut power_levels_content = room_power_levels.unwrap_or_default();
	power_levels_content.users.remove(&user_id);

	let event_id = self
		.services
		.rooms
		.timeline
		.build_and_append_pdu(
			PduBuilder {
				event_type: TimelineEventType::RoomPowerLevels,
				content: to_raw_value(&power_levels_content).expect("event is valid, we just created it"),
				unsigned: None,
				state_key: Some(String::new()),
				redacts: None,
				timestamp: None,
			},
			&user_id,
			&room_id,
			&state_lock,
		)
		.await?;

	Ok(RoomMessageEventContent::notice_markdown(format!(
		"User {user_id} demoted themselves to the room default power level in {room_id} - {event_id}"
	)))
}

#[admin_command]
pub(super) async fn make_user_admin(&self, user_id: String) -> Result<RoomMessageEventContent> {
	let user_id = parse_local_user_id(self.services, &user_id)?;

	assert!(
		self.services.globals.user_is_local(&user_id),
		"Parsed user_id must be a local user"
	);
	self.services.admin.make_user_admin(&user_id).await?;

	Ok(RoomMessageEventContent::notice_markdown(format!(
		"{user_id} has been granted admin privileges.",
	)))
}

#[admin_command]
pub(super) async fn put_room_tag(
	&self, user_id: String, room_id: Box<RoomId>, tag: String,
) -> Result<RoomMessageEventContent> {
	let user_id = parse_active_local_user_id(self.services, &user_id).await?;

	let mut tags_event = self
		.services
		.account_data
		.get_room(&room_id, &user_id, RoomAccountDataEventType::Tag)
		.await
		.unwrap_or(TagEvent {
			content: TagEventContent {
				tags: BTreeMap::new(),
			},
		});

	tags_event
		.content
		.tags
		.insert(tag.clone().into(), TagInfo::new());

	self.services
		.account_data
		.update(
			Some(&room_id),
			&user_id,
			RoomAccountDataEventType::Tag,
			&serde_json::to_value(tags_event).expect("to json value always works"),
		)
		.await?;

	Ok(RoomMessageEventContent::text_plain(format!(
		"Successfully updated room account data for {user_id} and room {room_id} with tag {tag}"
	)))
}

#[admin_command]
pub(super) async fn delete_room_tag(
	&self, user_id: String, room_id: Box<RoomId>, tag: String,
) -> Result<RoomMessageEventContent> {
	let user_id = parse_active_local_user_id(self.services, &user_id).await?;

	let mut tags_event = self
		.services
		.account_data
		.get_room(&room_id, &user_id, RoomAccountDataEventType::Tag)
		.await
		.unwrap_or(TagEvent {
			content: TagEventContent {
				tags: BTreeMap::new(),
			},
		});

	tags_event.content.tags.remove(&tag.clone().into());

	self.services
		.account_data
		.update(
			Some(&room_id),
			&user_id,
			RoomAccountDataEventType::Tag,
			&serde_json::to_value(tags_event).expect("to json value always works"),
		)
		.await?;

	Ok(RoomMessageEventContent::text_plain(format!(
		"Successfully updated room account data for {user_id} and room {room_id}, deleting room tag {tag}"
	)))
}

#[admin_command]
pub(super) async fn get_room_tags(&self, user_id: String, room_id: Box<RoomId>) -> Result<RoomMessageEventContent> {
	let user_id = parse_active_local_user_id(self.services, &user_id).await?;

	let tags_event = self
		.services
		.account_data
		.get_room(&room_id, &user_id, RoomAccountDataEventType::Tag)
		.await
		.unwrap_or(TagEvent {
			content: TagEventContent {
				tags: BTreeMap::new(),
			},
		});

	Ok(RoomMessageEventContent::notice_markdown(format!(
		"```\n{:#?}\n```",
		tags_event.content.tags
	)))
}

#[admin_command]
pub(super) async fn redact_event(&self, event_id: Box<EventId>) -> Result<RoomMessageEventContent> {
	let Ok(event) = self
		.services
		.rooms
		.timeline
		.get_non_outlier_pdu(&event_id)
		.await
	else {
		return Ok(RoomMessageEventContent::text_plain("Event does not exist in our database."));
	};

	if event.is_redacted() {
		return Ok(RoomMessageEventContent::text_plain("Event is already redacted."));
	}

	let room_id = event.room_id;
	let sender_user = event.sender;

	if !self.services.globals.user_is_local(&sender_user) {
		return Ok(RoomMessageEventContent::text_plain("This command only works on local users."));
	}

	let reason = format!(
		"The administrator(s) of {} has redacted this user's message.",
		self.services.globals.server_name()
	);

	let state_lock = self.services.rooms.state.mutex.lock(&room_id).await;

	let redaction_event_id = self
		.services
		.rooms
		.timeline
		.build_and_append_pdu(
			PduBuilder {
				event_type: TimelineEventType::RoomRedaction,
				content: to_raw_value(&RoomRedactionEventContent {
					redacts: Some(event.event_id.clone().into()),
					reason: Some(reason),
				})
				.expect("event is valid, we just created it"),
				unsigned: None,
				state_key: None,
				redacts: Some(event.event_id),
				timestamp: None,
			},
			&sender_user,
			&room_id,
			&state_lock,
		)
		.await?;

	drop(state_lock);

	Ok(RoomMessageEventContent::text_plain(format!(
		"Successfully redacted event. Redaction event ID: {redaction_event_id}"
	)))
}
