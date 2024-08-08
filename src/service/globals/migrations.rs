use conduit::{
	debug_info, debug_warn, error, info,
	result::NotFound,
	utils::{stream::TryIgnore, IterStream, ReadyExt},
	warn, Err, Error, Result,
};
use futures::{FutureExt, StreamExt};
use itertools::Itertools;
use ruma::{
	events::{push_rules::PushRulesEvent, room::member::MembershipState, GlobalAccountDataEventType},
	push::Ruleset,
	UserId,
};

use crate::{media, Services};

/// The current schema version.
/// - If database is opened at greater version we reject with error. The
///   software must be updated for backward-incompatible changes.
/// - If database is opened at lesser version we apply migrations up to this.
///   Note that named-feature migrations may also be performed when opening at
///   equal or lesser version. These are expected to be backward-compatible.
pub(crate) const DATABASE_VERSION: u64 = 13;

pub(crate) async fn migrations(services: &Services) -> Result<()> {
	let users_count = services.users.count().await;

	// Matrix resource ownership is based on the server name; changing it
	// requires recreating the database from scratch.
	if users_count > 0 {
		let conduit_user = &services.globals.server_user;

		if !services.users.exists(conduit_user).await {
			error!("The {} server user does not exist, and the database is not new.", conduit_user);
			return Err(Error::bad_database(
				"Cannot reuse an existing database after changing the server name, please delete the old one first.",
			));
		}
	}

	if users_count > 0 {
		migrate(services).await
	} else {
		fresh(services).await
	}
}

async fn fresh(services: &Services) -> Result<()> {
	let db = &services.db;
	let config = &services.server.config;

	services
		.globals
		.db
		.bump_database_version(DATABASE_VERSION)?;

	db["global"].insert(b"fix_bad_double_separator_in_state_cache", &[]);
	db["global"].insert(b"retroactively_fix_bad_data_from_roomuserid_joined", &[]);

	// Create the admin room and server user on first run
	crate::admin::create_admin_room(services).await?;

	warn!(
		"Created new {} database with version {DATABASE_VERSION}",
		config.database_backend,
	);

	Ok(())
}

/// Apply any migrations
async fn migrate(services: &Services) -> Result<()> {
	let db = &services.db;
	let config = &services.server.config;

	if services.globals.db.database_version().await < 11 {
		return Err!(Database(
			"Database schema version {} is no longer supported",
			services.globals.db.database_version().await
		));
	}

	if services.globals.db.database_version().await < 12 {
		db_lt_12(services).await?;
	}

	// This migration can be reused as-is anytime the server-default rules are
	// updated.
	if services.globals.db.database_version().await < 13 {
		db_lt_13(services).await?;
	}

	if db["global"].qry("feat_sha256_media").await.is_not_found() {
		media::migrations::migrate_sha256_media(services).await?;
	} else if config.media_startup_check {
		media::migrations::checkup_sha256_media(services).await?;
	}

	if db["global"]
		.qry("fix_bad_double_separator_in_state_cache")
		.await
		.is_not_found()
	{
		fix_bad_double_separator_in_state_cache(services).await?;
	}

	if db["global"]
		.qry("retroactively_fix_bad_data_from_roomuserid_joined")
		.await
		.is_not_found()
	{
		retroactively_fix_bad_data_from_roomuserid_joined(services).await?;
	}

	assert_eq!(
		services.globals.db.database_version().await,
		DATABASE_VERSION,
		"Failed asserting local database version {} is equal to known latest conduwuit database version {}",
		services.globals.db.database_version().await,
		DATABASE_VERSION,
	);

	{
		let patterns = services.globals.forbidden_usernames();
		if !patterns.is_empty() {
			services
				.users
				.stream()
				.filter(|user_id| services.users.is_active_local(user_id))
				.ready_for_each(|user_id| {
					let matches = patterns.matches(user_id.localpart());
					if matches.matched_any() {
						warn!(
							"User {} matches the following forbidden username patterns: {}",
							user_id.to_string(),
							matches
								.into_iter()
								.map(|x| &patterns.patterns()[x])
								.join(", ")
						);
					}
				})
				.await;
		}
	}

	{
		let patterns = services.globals.forbidden_alias_names();
		if !patterns.is_empty() {
			for room_id in services
				.rooms
				.metadata
				.iter_ids()
				.map(ToOwned::to_owned)
				.collect::<Vec<_>>()
				.await
			{
				services
					.rooms
					.alias
					.local_aliases_for_room(&room_id)
					.ready_for_each(|room_alias| {
						let matches = patterns.matches(room_alias.alias());
						if matches.matched_any() {
							warn!(
								"Room with alias {} ({}) matches the following forbidden room name patterns: {}",
								room_alias,
								&room_id,
								matches
									.into_iter()
									.map(|x| &patterns.patterns()[x])
									.join(", ")
							);
						}
					})
					.await;
			}
		}
	}

	info!(
		"Loaded {} database with schema version {DATABASE_VERSION}",
		config.database_backend,
	);

	Ok(())
}

async fn db_lt_12(services: &Services) -> Result<()> {
	let config = &services.server.config;

	for username in &services
		.users
		.list_local_users()
		.map(UserId::to_owned)
		.collect::<Vec<_>>()
		.await
	{
		let user = match UserId::parse_with_server_name(username.as_str(), &config.server_name) {
			Ok(u) => u,
			Err(e) => {
				warn!("Invalid username {username}: {e}");
				continue;
			},
		};

		let raw_rules_list = services
			.account_data
			.get(None, &user, GlobalAccountDataEventType::PushRules.to_string().into())
			.await
			.expect("Username is invalid");

		let mut account_data = serde_json::from_str::<PushRulesEvent>(raw_rules_list.get()).unwrap();
		let rules_list = &mut account_data.content.global;

		//content rule
		{
			let content_rule_transformation = [".m.rules.contains_user_name", ".m.rule.contains_user_name"];

			let rule = rules_list.content.get(content_rule_transformation[0]);
			if rule.is_some() {
				let mut rule = rule.unwrap().clone();
				content_rule_transformation[1].clone_into(&mut rule.rule_id);
				rules_list
					.content
					.shift_remove(content_rule_transformation[0]);
				rules_list.content.insert(rule);
			}
		}

		//underride rules
		{
			let underride_rule_transformation = [
				[".m.rules.call", ".m.rule.call"],
				[".m.rules.room_one_to_one", ".m.rule.room_one_to_one"],
				[".m.rules.encrypted_room_one_to_one", ".m.rule.encrypted_room_one_to_one"],
				[".m.rules.message", ".m.rule.message"],
				[".m.rules.encrypted", ".m.rule.encrypted"],
			];

			for transformation in underride_rule_transformation {
				let rule = rules_list.underride.get(transformation[0]);
				if let Some(rule) = rule {
					let mut rule = rule.clone();
					transformation[1].clone_into(&mut rule.rule_id);
					rules_list.underride.shift_remove(transformation[0]);
					rules_list.underride.insert(rule);
				}
			}
		}

		services
			.account_data
			.update(
				None,
				&user,
				GlobalAccountDataEventType::PushRules.to_string().into(),
				&serde_json::to_value(account_data).expect("to json value always works"),
			)
			.await?;
	}

	services.globals.db.bump_database_version(12)?;
	info!("Migration: 11 -> 12 finished");
	Ok(())
}

async fn db_lt_13(services: &Services) -> Result<()> {
	let config = &services.server.config;

	for username in &services
		.users
		.list_local_users()
		.map(UserId::to_owned)
		.collect::<Vec<_>>()
		.await
	{
		let user = match UserId::parse_with_server_name(username.as_str(), &config.server_name) {
			Ok(u) => u,
			Err(e) => {
				warn!("Invalid username {username}: {e}");
				continue;
			},
		};

		let raw_rules_list = services
			.account_data
			.get(None, &user, GlobalAccountDataEventType::PushRules.to_string().into())
			.await
			.expect("Username is invalid");

		let mut account_data = serde_json::from_str::<PushRulesEvent>(raw_rules_list.get()).unwrap();

		let user_default_rules = Ruleset::server_default(&user);
		account_data
			.content
			.global
			.update_with_server_default(user_default_rules);

		services
			.account_data
			.update(
				None,
				&user,
				GlobalAccountDataEventType::PushRules.to_string().into(),
				&serde_json::to_value(account_data).expect("to json value always works"),
			)
			.await?;
	}

	services.globals.db.bump_database_version(13)?;
	info!("Migration: 12 -> 13 finished");
	Ok(())
}

async fn fix_bad_double_separator_in_state_cache(services: &Services) -> Result<()> {
	warn!("Fixing bad double separator in state_cache roomuserid_joined");

	let db = &services.db;
	let roomuserid_joined = &db["roomuserid_joined"];
	let _cork = db.cork_and_sync();

	let mut iter_count: usize = 0;
	roomuserid_joined
		.raw_stream()
		.ignore_err()
		.ready_for_each(|(key, value)| {
			let mut key = key.to_vec();
			iter_count = iter_count.saturating_add(1);
			debug_info!(%iter_count);
			let first_sep_index = key
				.iter()
				.position(|&i| i == 0xFF)
				.expect("found 0xFF delim");

			if key
				.iter()
				.get(first_sep_index..=first_sep_index.saturating_add(1))
				.copied()
				.collect_vec()
				== vec![0xFF, 0xFF]
			{
				debug_warn!("Found bad key: {key:?}");
				roomuserid_joined.remove(&key);

				key.remove(first_sep_index);
				debug_warn!("Fixed key: {key:?}");
				roomuserid_joined.insert(&key, value);
			}
		})
		.await;

	db.db.cleanup()?;
	db["global"].insert(b"fix_bad_double_separator_in_state_cache", &[]);

	info!("Finished fixing");
	Ok(())
}

async fn retroactively_fix_bad_data_from_roomuserid_joined(services: &Services) -> Result<()> {
	warn!("Retroactively fixing bad data from broken roomuserid_joined");

	let db = &services.db;
	let _cork = db.cork_and_sync();

	let room_ids = services
		.rooms
		.metadata
		.iter_ids()
		.map(ToOwned::to_owned)
		.collect::<Vec<_>>()
		.await;

	for room_id in &room_ids {
		debug_info!("Fixing room {room_id}");

		let users_in_room = services
			.rooms
			.state_cache
			.room_members(room_id)
			.collect::<Vec<_>>()
			.await;

		let joined_members = users_in_room
			.iter()
			.stream()
			.filter(|user_id| {
				services
					.rooms
					.state_accessor
					.get_member(room_id, user_id)
					.map(|member| member.map_or(false, |member| member.membership == MembershipState::Join))
			})
			.collect::<Vec<_>>()
			.await;

		let non_joined_members = users_in_room
			.iter()
			.stream()
			.filter(|user_id| {
				services
					.rooms
					.state_accessor
					.get_member(room_id, user_id)
					.map(|member| member.map_or(false, |member| member.membership == MembershipState::Join))
			})
			.collect::<Vec<_>>()
			.await;

		for user_id in joined_members {
			debug_info!("User is joined, marking as joined");
			services.rooms.state_cache.mark_as_joined(user_id, room_id);
		}

		for user_id in non_joined_members {
			debug_info!("User is left or banned, marking as left");
			services.rooms.state_cache.mark_as_left(user_id, room_id);
		}
	}

	for room_id in &room_ids {
		debug_info!(
			"Updating joined count for room {room_id} to fix servers in room after correcting membership states"
		);

		services
			.rooms
			.state_cache
			.update_joined_count(room_id)
			.await;
	}

	db.db.cleanup()?;
	db["global"].insert(b"retroactively_fix_bad_data_from_roomuserid_joined", &[]);

	info!("Finished fixing");
	Ok(())
}
