use std::{
	collections::{HashMap, HashSet},
	ffi::{OsStr, OsString},
	fs::{self},
	io::Write,
	mem::size_of,
	path::PathBuf,
	sync::Arc,
	time::Instant,
};

use conduit::{debug, debug_info, debug_warn, error, info, utils, warn, Config, Error, Result};
use itertools::Itertools;
use ruma::{
	events::{push_rules::PushRulesEvent, room::member::MembershipState, GlobalAccountDataEventType},
	push::Ruleset,
	EventId, OwnedRoomId, RoomId, UserId,
};

use crate::Services;

/// The current schema version.
/// - If database is opened at greater version we reject with error. The
///   software must be updated for backward-incompatible changes.
/// - If database is opened at lesser version we apply migrations up to this.
///   Note that named-feature migrations may also be performed when opening at
///   equal or lesser version. These are expected to be backward-compatible.
const DATABASE_VERSION: u64 = 13;

pub(crate) async fn migrations(services: &Services) -> Result<()> {
	// Matrix resource ownership is based on the server name; changing it
	// requires recreating the database from scratch.
	if services.users.count()? > 0 {
		let conduit_user = &services.globals.server_user;

		if !services.users.exists(conduit_user)? {
			error!("The {} server user does not exist, and the database is not new.", conduit_user);
			return Err(Error::bad_database(
				"Cannot reuse an existing database after changing the server name, please delete the old one first.",
			));
		}
	}

	if services.users.count()? > 0 {
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

	db["global"].insert(b"fix_bad_double_separator_in_state_cache", &[])?;
	db["global"].insert(b"retroactively_fix_bad_data_from_roomuserid_joined", &[])?;

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

	if services.globals.db.database_version()? < 1 {
		db_lt_1(services).await?;
	}

	if services.globals.db.database_version()? < 2 {
		db_lt_2(services).await?;
	}

	if services.globals.db.database_version()? < 3 {
		db_lt_3(services).await?;
	}

	if services.globals.db.database_version()? < 4 {
		db_lt_4(services).await?;
	}

	if services.globals.db.database_version()? < 5 {
		db_lt_5(services).await?;
	}

	if services.globals.db.database_version()? < 6 {
		db_lt_6(services).await?;
	}

	if services.globals.db.database_version()? < 7 {
		db_lt_7(services).await?;
	}

	if services.globals.db.database_version()? < 8 {
		db_lt_8(services).await?;
	}

	if services.globals.db.database_version()? < 9 {
		db_lt_9(services).await?;
	}

	if services.globals.db.database_version()? < 10 {
		db_lt_10(services).await?;
	}

	if services.globals.db.database_version()? < 11 {
		db_lt_11(services).await?;
	}

	if services.globals.db.database_version()? < 12 {
		db_lt_12(services).await?;
	}

	// This migration can be reused as-is anytime the server-default rules are
	// updated.
	if services.globals.db.database_version()? < 13 {
		db_lt_13(services).await?;
	}

	if db["global"].get(b"feat_sha256_media")?.is_none() {
		migrate_sha256_media(services).await?;
	} else if config.media_startup_check {
		checkup_sha256_media(services).await?;
	}

	if db["global"]
		.get(b"fix_bad_double_separator_in_state_cache")?
		.is_none()
	{
		fix_bad_double_separator_in_state_cache(services).await?;
	}

	if db["global"]
		.get(b"retroactively_fix_bad_data_from_roomuserid_joined")?
		.is_none()
	{
		retroactively_fix_bad_data_from_roomuserid_joined(services).await?;
	}

	assert_eq!(
		services.globals.db.database_version().unwrap(),
		DATABASE_VERSION,
		"Failed asserting local database version {} is equal to known latest conduwuit database version {}",
		services.globals.db.database_version().unwrap(),
		DATABASE_VERSION,
	);

	{
		let patterns = services.globals.forbidden_usernames();
		if !patterns.is_empty() {
			for user_id in services
				.users
				.iter()
				.filter_map(Result::ok)
				.filter(|user| !services.users.is_deactivated(user).unwrap_or(true))
				.filter(|user| user.server_name() == config.server_name)
			{
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
			}
		}
	}

	{
		let patterns = services.globals.forbidden_alias_names();
		if !patterns.is_empty() {
			for address in services.rooms.metadata.iter_ids() {
				let room_id = address?;
				let room_aliases = services.rooms.alias.local_aliases_for_room(&room_id);
				for room_alias_result in room_aliases {
					let room_alias = room_alias_result?;
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
				}
			}
		}
	}

	info!(
		"Loaded {} database with schema version {DATABASE_VERSION}",
		config.database_backend,
	);

	Ok(())
}

async fn db_lt_1(services: &Services) -> Result<()> {
	let db = &services.db;

	let roomserverids = &db["roomserverids"];
	let serverroomids = &db["serverroomids"];
	for (roomserverid, _) in roomserverids.iter() {
		let mut parts = roomserverid.split(|&b| b == 0xFF);
		let room_id = parts.next().expect("split always returns one element");
		let Some(servername) = parts.next() else {
			error!("Migration: Invalid roomserverid in db.");
			continue;
		};
		let mut serverroomid = servername.to_vec();
		serverroomid.push(0xFF);
		serverroomid.extend_from_slice(room_id);

		serverroomids.insert(&serverroomid, &[])?;
	}

	services.globals.db.bump_database_version(1)?;
	info!("Migration: 0 -> 1 finished");
	Ok(())
}

async fn db_lt_2(services: &Services) -> Result<()> {
	let db = &services.db;

	// We accidentally inserted hashed versions of "" into the db instead of just ""
	let userid_password = &db["roomserverids"];
	for (userid, password) in userid_password.iter() {
		let empty_pass = utils::hash::password("").expect("our own password to be properly hashed");
		let password = std::str::from_utf8(&password).expect("password is valid utf-8");
		let empty_hashed_password = utils::hash::verify_password(password, &empty_pass).is_ok();
		if empty_hashed_password {
			userid_password.insert(&userid, b"")?;
		}
	}

	services.globals.db.bump_database_version(2)?;
	info!("Migration: 1 -> 2 finished");
	Ok(())
}

async fn db_lt_3(services: &Services) -> Result<()> {
	let db = &services.db;

	// Move media to filesystem
	let mediaid_file = &db["mediaid_file"];
	for (key, content) in mediaid_file.iter() {
		if content.is_empty() {
			continue;
		}

		#[allow(deprecated)]
		let path = services.media.get_media_file(&key);
		let mut file = fs::File::create(path)?;
		file.write_all(&content)?;
		mediaid_file.insert(&key, &[])?;
	}

	services.globals.db.bump_database_version(3)?;
	info!("Migration: 2 -> 3 finished");
	Ok(())
}

async fn db_lt_4(services: &Services) -> Result<()> {
	let config = &services.server.config;

	// Add federated users to services as deactivated
	for our_user in services.users.iter() {
		let our_user = our_user?;
		if services.users.is_deactivated(&our_user)? {
			continue;
		}
		for room in services.rooms.state_cache.rooms_joined(&our_user) {
			for user in services.rooms.state_cache.room_members(&room?) {
				let user = user?;
				if user.server_name() != config.server_name {
					info!(?user, "Migration: creating user");
					services.users.create(&user, None)?;
				}
			}
		}
	}

	services.globals.db.bump_database_version(4)?;
	info!("Migration: 3 -> 4 finished");
	Ok(())
}

async fn db_lt_5(services: &Services) -> Result<()> {
	let db = &services.db;

	// Upgrade user data store
	let roomuserdataid_accountdata = &db["roomuserdataid_accountdata"];
	let roomusertype_roomuserdataid = &db["roomusertype_roomuserdataid"];
	for (roomuserdataid, _) in roomuserdataid_accountdata.iter() {
		let mut parts = roomuserdataid.split(|&b| b == 0xFF);
		let room_id = parts.next().unwrap();
		let user_id = parts.next().unwrap();
		let event_type = roomuserdataid.rsplit(|&b| b == 0xFF).next().unwrap();

		let mut key = room_id.to_vec();
		key.push(0xFF);
		key.extend_from_slice(user_id);
		key.push(0xFF);
		key.extend_from_slice(event_type);

		roomusertype_roomuserdataid.insert(&key, &roomuserdataid)?;
	}

	services.globals.db.bump_database_version(5)?;
	info!("Migration: 4 -> 5 finished");
	Ok(())
}

async fn db_lt_6(services: &Services) -> Result<()> {
	let db = &services.db;

	// Set room member count
	let roomid_shortstatehash = &db["roomid_shortstatehash"];
	for (roomid, _) in roomid_shortstatehash.iter() {
		let string = utils::string_from_bytes(&roomid).unwrap();
		let room_id = <&RoomId>::try_from(string.as_str()).unwrap();
		services.rooms.state_cache.update_joined_count(room_id)?;
	}

	services.globals.db.bump_database_version(6)?;
	info!("Migration: 5 -> 6 finished");
	Ok(())
}

async fn db_lt_7(services: &Services) -> Result<()> {
	let db = &services.db;

	// Upgrade state store
	let mut last_roomstates: HashMap<OwnedRoomId, u64> = HashMap::new();
	let mut current_sstatehash: Option<u64> = None;
	let mut current_room = None;
	let mut current_state = HashSet::new();

	let handle_state = |current_sstatehash: u64,
	                    current_room: &RoomId,
	                    current_state: HashSet<_>,
	                    last_roomstates: &mut HashMap<_, _>| {
		let last_roomsstatehash = last_roomstates.get(current_room);

		let states_parents = last_roomsstatehash.map_or_else(
			|| Ok(Vec::new()),
			|&last_roomsstatehash| {
				services
					.rooms
					.state_compressor
					.load_shortstatehash_info(last_roomsstatehash)
			},
		)?;

		let (statediffnew, statediffremoved) = if let Some(parent_stateinfo) = states_parents.last() {
			let statediffnew = current_state
				.difference(&parent_stateinfo.1)
				.copied()
				.collect::<HashSet<_>>();

			let statediffremoved = parent_stateinfo
				.1
				.difference(&current_state)
				.copied()
				.collect::<HashSet<_>>();

			(statediffnew, statediffremoved)
		} else {
			(current_state, HashSet::new())
		};

		services.rooms.state_compressor.save_state_from_diff(
			current_sstatehash,
			Arc::new(statediffnew),
			Arc::new(statediffremoved),
			2, // every state change is 2 event changes on average
			states_parents,
		)?;

		/*
		let mut tmp = services.rooms.load_shortstatehash_info(&current_sstatehash)?;
		let state = tmp.pop().unwrap();
		println!(
			"{}\t{}{:?}: {:?} + {:?} - {:?}",
			current_room,
			"  ".repeat(tmp.len()),
			utils::u64_from_bytes(&current_sstatehash).unwrap(),
			tmp.last().map(|b| utils::u64_from_bytes(&b.0).unwrap()),
			state
				.2
				.iter()
				.map(|b| utils::u64_from_bytes(&b[size_of::<u64>()..]).unwrap())
				.collect::<Vec<_>>(),
			state
				.3
				.iter()
				.map(|b| utils::u64_from_bytes(&b[size_of::<u64>()..]).unwrap())
				.collect::<Vec<_>>()
		);
		*/

		Ok::<_, Error>(())
	};

	let stateid_shorteventid = &db["stateid_shorteventid"];
	let shorteventid_eventid = &db["shorteventid_eventid"];
	for (k, seventid) in stateid_shorteventid.iter() {
		let sstatehash = utils::u64_from_bytes(&k[0..size_of::<u64>()]).expect("number of bytes is correct");
		let sstatekey = k[size_of::<u64>()..].to_vec();
		if Some(sstatehash) != current_sstatehash {
			if let Some(current_sstatehash) = current_sstatehash {
				handle_state(
					current_sstatehash,
					current_room.as_deref().unwrap(),
					current_state,
					&mut last_roomstates,
				)?;
				last_roomstates.insert(current_room.clone().unwrap(), current_sstatehash);
			}
			current_state = HashSet::new();
			current_sstatehash = Some(sstatehash);

			let event_id = shorteventid_eventid.get(&seventid).unwrap().unwrap();
			let string = utils::string_from_bytes(&event_id).unwrap();
			let event_id = <&EventId>::try_from(string.as_str()).unwrap();
			let pdu = services.rooms.timeline.get_pdu(event_id).unwrap().unwrap();

			if Some(&pdu.room_id) != current_room.as_ref() {
				current_room = Some(pdu.room_id.clone());
			}
		}

		let mut val = sstatekey;
		val.extend_from_slice(&seventid);
		current_state.insert(val.try_into().expect("size is correct"));
	}

	if let Some(current_sstatehash) = current_sstatehash {
		handle_state(
			current_sstatehash,
			current_room.as_deref().unwrap(),
			current_state,
			&mut last_roomstates,
		)?;
	}

	services.globals.db.bump_database_version(7)?;
	info!("Migration: 6 -> 7 finished");
	Ok(())
}

async fn db_lt_8(services: &Services) -> Result<()> {
	let db = &services.db;

	let roomid_shortstatehash = &db["roomid_shortstatehash"];
	let roomid_shortroomid = &db["roomid_shortroomid"];
	let pduid_pdu = &db["pduid_pdu"];
	let eventid_pduid = &db["eventid_pduid"];

	// Generate short room ids for all rooms
	for (room_id, _) in roomid_shortstatehash.iter() {
		let shortroomid = services.globals.next_count()?.to_be_bytes();
		roomid_shortroomid.insert(&room_id, &shortroomid)?;
		info!("Migration: 8");
	}
	// Update pduids db layout
	let batch = pduid_pdu
		.iter()
		.filter_map(|(key, v)| {
			if !key.starts_with(b"!") {
				return None;
			}
			let mut parts = key.splitn(2, |&b| b == 0xFF);
			let room_id = parts.next().unwrap();
			let count = parts.next().unwrap();

			let short_room_id = roomid_shortroomid
				.get(room_id)
				.unwrap()
				.expect("shortroomid should exist");

			let mut new_key = short_room_id.to_vec();
			new_key.extend_from_slice(count);

			Some(database::OwnedKeyVal(new_key, v))
		})
		.collect::<Vec<_>>();

	pduid_pdu.insert_batch(batch.iter().map(database::KeyVal::from))?;

	let batch2 = eventid_pduid
		.iter()
		.filter_map(|(k, value)| {
			if !value.starts_with(b"!") {
				return None;
			}
			let mut parts = value.splitn(2, |&b| b == 0xFF);
			let room_id = parts.next().unwrap();
			let count = parts.next().unwrap();

			let short_room_id = roomid_shortroomid
				.get(room_id)
				.unwrap()
				.expect("shortroomid should exist");

			let mut new_value = short_room_id.to_vec();
			new_value.extend_from_slice(count);

			Some(database::OwnedKeyVal(k, new_value))
		})
		.collect::<Vec<_>>();

	eventid_pduid.insert_batch(batch2.iter().map(database::KeyVal::from))?;

	services.globals.db.bump_database_version(8)?;
	info!("Migration: 7 -> 8 finished");
	Ok(())
}

async fn db_lt_9(services: &Services) -> Result<()> {
	let db = &services.db;

	let tokenids = &db["tokenids"];
	let roomid_shortroomid = &db["roomid_shortroomid"];

	// Update tokenids db layout
	let mut iter = tokenids
		.iter()
		.filter_map(|(key, _)| {
			if !key.starts_with(b"!") {
				return None;
			}
			let mut parts = key.splitn(4, |&b| b == 0xFF);
			let room_id = parts.next().unwrap();
			let word = parts.next().unwrap();
			let _pdu_id_room = parts.next().unwrap();
			let pdu_id_count = parts.next().unwrap();

			let short_room_id = roomid_shortroomid
				.get(room_id)
				.unwrap()
				.expect("shortroomid should exist");
			let mut new_key = short_room_id.to_vec();
			new_key.extend_from_slice(word);
			new_key.push(0xFF);
			new_key.extend_from_slice(pdu_id_count);
			Some(database::OwnedKeyVal(new_key, Vec::<u8>::new()))
		})
		.peekable();

	while iter.peek().is_some() {
		let batch = iter.by_ref().take(1000).collect::<Vec<_>>();
		tokenids.insert_batch(batch.iter().map(database::KeyVal::from))?;
		debug!("Inserted smaller batch");
	}

	info!("Deleting starts");

	let batch2: Vec<_> = tokenids
		.iter()
		.filter_map(|(key, _)| {
			if key.starts_with(b"!") {
				Some(key)
			} else {
				None
			}
		})
		.collect();

	for key in batch2 {
		tokenids.remove(&key)?;
	}

	services.globals.db.bump_database_version(9)?;
	info!("Migration: 8 -> 9 finished");
	Ok(())
}

async fn db_lt_10(services: &Services) -> Result<()> {
	let db = &services.db;

	let statekey_shortstatekey = &db["statekey_shortstatekey"];
	let shortstatekey_statekey = &db["shortstatekey_statekey"];

	// Add other direction for shortstatekeys
	for (statekey, shortstatekey) in statekey_shortstatekey.iter() {
		shortstatekey_statekey.insert(&shortstatekey, &statekey)?;
	}

	// Force E2EE device list updates so we can send them over federation
	for user_id in services.users.iter().filter_map(Result::ok) {
		services.users.mark_device_key_update(&user_id)?;
	}

	services.globals.db.bump_database_version(10)?;
	info!("Migration: 9 -> 10 finished");
	Ok(())
}

#[allow(unreachable_code)]
async fn db_lt_11(services: &Services) -> Result<()> {
	error!("Dropping a column to clear data is not implemented yet.");
	//let userdevicesessionid_uiaarequest = &db["userdevicesessionid_uiaarequest"];
	//userdevicesessionid_uiaarequest.clear()?;

	services.globals.db.bump_database_version(11)?;
	info!("Migration: 10 -> 11 finished");
	Ok(())
}

async fn db_lt_12(services: &Services) -> Result<()> {
	let config = &services.server.config;

	for username in services.users.list_local_users()? {
		let user = match UserId::parse_with_server_name(username.clone(), &config.server_name) {
			Ok(u) => u,
			Err(e) => {
				warn!("Invalid username {username}: {e}");
				continue;
			},
		};

		let raw_rules_list = services
			.account_data
			.get(None, &user, GlobalAccountDataEventType::PushRules.to_string().into())
			.unwrap()
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

		services.account_data.update(
			None,
			&user,
			GlobalAccountDataEventType::PushRules.to_string().into(),
			&serde_json::to_value(account_data).expect("to json value always works"),
		)?;
	}

	services.globals.db.bump_database_version(12)?;
	info!("Migration: 11 -> 12 finished");
	Ok(())
}

async fn db_lt_13(services: &Services) -> Result<()> {
	let config = &services.server.config;

	for username in services.users.list_local_users()? {
		let user = match UserId::parse_with_server_name(username.clone(), &config.server_name) {
			Ok(u) => u,
			Err(e) => {
				warn!("Invalid username {username}: {e}");
				continue;
			},
		};

		let raw_rules_list = services
			.account_data
			.get(None, &user, GlobalAccountDataEventType::PushRules.to_string().into())
			.unwrap()
			.expect("Username is invalid");

		let mut account_data = serde_json::from_str::<PushRulesEvent>(raw_rules_list.get()).unwrap();

		let user_default_rules = Ruleset::server_default(&user);
		account_data
			.content
			.global
			.update_with_server_default(user_default_rules);

		services.account_data.update(
			None,
			&user,
			GlobalAccountDataEventType::PushRules.to_string().into(),
			&serde_json::to_value(account_data).expect("to json value always works"),
		)?;
	}

	services.globals.db.bump_database_version(13)?;
	info!("Migration: 12 -> 13 finished");
	Ok(())
}

/// Migrates a media directory from legacy base64 file names to sha2 file names.
/// All errors are fatal. Upon success the database is keyed to not perform this
/// again.
async fn migrate_sha256_media(services: &Services) -> Result<()> {
	let db = &services.db;
	let config = &services.server.config;

	warn!("Migrating legacy base64 file names to sha256 file names");
	let mediaid_file = &db["mediaid_file"];

	// Move old media files to new names
	let mut changes = Vec::<(PathBuf, PathBuf)>::new();
	for (key, _) in mediaid_file.iter() {
		let old = services.media.get_media_file_b64(&key);
		let new = services.media.get_media_file_sha256(&key);
		debug!(?key, ?old, ?new, num = changes.len(), "change");
		changes.push((old, new));
	}
	// move the file to the new location
	for (old_path, path) in changes {
		if old_path.exists() {
			tokio::fs::rename(&old_path, &path).await?;
			if config.media_compat_file_link {
				tokio::fs::symlink(&path, &old_path).await?;
			}
		}
	}

	// Apply fix from when sha256_media was backward-incompat and bumped the schema
	// version from 13 to 14. For users satisfying these conditions we can go back.
	if services.globals.db.database_version()? == 14 && DATABASE_VERSION == 13 {
		services.globals.db.bump_database_version(13)?;
	}

	db["global"].insert(b"feat_sha256_media", &[])?;
	info!("Finished applying sha256_media");
	Ok(())
}

/// Check is run on startup for prior-migrated media directories. This handles:
/// - Going back and forth to non-sha256 legacy binaries (e.g. upstream).
/// - Deletion of artifacts in the media directory which will then fall out of
///   sync with the database.
async fn checkup_sha256_media(services: &Services) -> Result<()> {
	use crate::media::encode_key;

	debug!("Checking integrity of media directory");
	let db = &services.db;
	let media = &services.media;
	let config = &services.server.config;
	let mediaid_file = &db["mediaid_file"];
	let mediaid_user = &db["mediaid_user"];
	let dbs = (mediaid_file, mediaid_user);
	let timer = Instant::now();

	let dir = media.get_media_dir();
	let files: HashSet<OsString> = fs::read_dir(dir)?
		.filter_map(|ent| ent.map_or(None, |ent| Some(ent.path().into_os_string())))
		.collect();

	for key in media.db.get_all_media_keys() {
		let new_path = media.get_media_file_sha256(&key).into_os_string();
		let old_path = media.get_media_file_b64(&key).into_os_string();
		if let Err(e) = handle_media_check(&dbs, config, &files, &key, &new_path, &old_path).await {
			error!(
				media_id = ?encode_key(&key), ?new_path, ?old_path,
				"Failed to resolve media check failure: {e}"
			);
		}
	}

	debug_info!(
		elapsed = ?timer.elapsed(),
		"Finished checking media directory"
	);

	Ok(())
}

async fn handle_media_check(
	dbs: &(&Arc<database::Map>, &Arc<database::Map>), config: &Config, files: &HashSet<OsString>, key: &[u8],
	new_path: &OsStr, old_path: &OsStr,
) -> Result<()> {
	use crate::media::encode_key;

	let (mediaid_file, mediaid_user) = dbs;

	let new_exists = files.contains(new_path);
	let old_exists = files.contains(old_path);
	let old_is_symlink = || async {
		tokio::fs::symlink_metadata(old_path)
			.await
			.map_or(false, |md| md.is_symlink())
	};

	if !old_exists && !new_exists {
		error!(
			media_id = ?encode_key(key), ?new_path, ?old_path,
			"Media is missing at all paths. Removing from database..."
		);

		mediaid_file.remove(key)?;
		mediaid_user.remove(key)?;
	}

	if config.media_compat_file_link && !old_exists && new_exists {
		debug_warn!(
			media_id = ?encode_key(key), ?new_path, ?old_path,
			"Media found but missing legacy link. Fixing..."
		);

		tokio::fs::symlink(&new_path, &old_path).await?;
	}

	if config.media_compat_file_link && !new_exists && old_exists {
		debug_warn!(
			media_id = ?encode_key(key), ?new_path, ?old_path,
			"Legacy media found without sha256 migration. Fixing..."
		);

		debug_assert!(
			old_is_symlink().await,
			"Legacy media not expected to be a symlink without an existing sha256 migration."
		);

		tokio::fs::rename(&old_path, &new_path).await?;
		tokio::fs::symlink(&new_path, &old_path).await?;
	}

	if !config.media_compat_file_link && old_exists && old_is_symlink().await {
		debug_warn!(
			media_id = ?encode_key(key), ?new_path, ?old_path,
			"Legacy link found but compat disabled. Cleansing symlink..."
		);

		debug_assert!(
			new_exists,
			"sha256 migration into new file expected prior to cleaning legacy symlink here."
		);

		tokio::fs::remove_file(&old_path).await?;
	}

	Ok(())
}

async fn fix_bad_double_separator_in_state_cache(services: &Services) -> Result<()> {
	warn!("Fixing bad double separator in state_cache roomuserid_joined");

	let db = &services.db;
	let roomuserid_joined = &db["roomuserid_joined"];
	let _cork = db.cork_and_sync();

	let mut iter_count: usize = 0;
	for (mut key, value) in roomuserid_joined.iter() {
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
			roomuserid_joined.remove(&key)?;

			key.remove(first_sep_index);
			debug_warn!("Fixed key: {key:?}");
			roomuserid_joined.insert(&key, &value)?;
		}
	}

	db.db.cleanup()?;
	db["global"].insert(b"fix_bad_double_separator_in_state_cache", &[])?;

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
		.filter_map(Result::ok)
		.collect_vec();

	for room_id in room_ids.clone() {
		debug_info!("Fixing room {room_id}");

		let users_in_room = services
			.rooms
			.state_cache
			.room_members(&room_id)
			.filter_map(Result::ok)
			.collect_vec();

		let joined_members = users_in_room
			.iter()
			.filter(|user_id| {
				services
					.rooms
					.state_accessor
					.get_member(&room_id, user_id)
					.unwrap_or(None)
					.map_or(false, |membership| membership.membership == MembershipState::Join)
			})
			.collect_vec();

		let non_joined_members = users_in_room
			.iter()
			.filter(|user_id| {
				services
					.rooms
					.state_accessor
					.get_member(&room_id, user_id)
					.unwrap_or(None)
					.map_or(false, |membership| {
						membership.membership == MembershipState::Leave || membership.membership == MembershipState::Ban
					})
			})
			.collect_vec();

		for user_id in joined_members {
			debug_info!("User is joined, marking as joined");
			services
				.rooms
				.state_cache
				.mark_as_joined(user_id, &room_id)?;
		}

		for user_id in non_joined_members {
			debug_info!("User is left or banned, marking as left");
			services.rooms.state_cache.mark_as_left(user_id, &room_id)?;
		}
	}

	for room_id in room_ids {
		debug_info!(
			"Updating joined count for room {room_id} to fix servers in room after correcting membership states"
		);

		services.rooms.state_cache.update_joined_count(&room_id)?;
	}

	db.db.cleanup()?;
	db["global"].insert(b"retroactively_fix_bad_data_from_roomuserid_joined", &[])?;

	info!("Finished fixing");
	Ok(())
}
