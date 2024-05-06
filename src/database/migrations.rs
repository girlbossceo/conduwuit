use std::{
	collections::{HashMap, HashSet},
	fs::{self},
	io::Write,
	mem::size_of,
	sync::Arc,
};

use argon2::{password_hash::SaltString, PasswordHasher, PasswordVerifier};
use itertools::Itertools;
use rand::thread_rng;
use ruma::{
	events::{push_rules::PushRulesEvent, GlobalAccountDataEventType},
	push::Ruleset,
	EventId, OwnedRoomId, RoomId, UserId,
};
use tracing::{debug, error, info, warn};

use super::KeyValueDatabase;
use crate::{services, utils, Config, Error, Result};

pub(crate) async fn migrations(db: &KeyValueDatabase, config: &Config) -> Result<()> {
	// Matrix resource ownership is based on the server name; changing it
	// requires recreating the database from scratch.
	if services().users.count()? > 0 {
		let conduit_user =
			UserId::parse_with_server_name("conduit", &config.server_name).expect("@conduit:server_name is valid");

		if !services().users.exists(&conduit_user)? {
			error!("The {} server user does not exist, and the database is not new.", conduit_user);
			return Err(Error::bad_database(
				"Cannot reuse an existing database after changing the server name, please delete the old one first.",
			));
		}
	}

	// If the database has any data, perform data migrations before starting
	// do not increment the db version if the user is not using sha256_media
	let latest_database_version = if cfg!(feature = "sha256_media") {
		14
	} else {
		13
	};

	if services().users.count()? > 0 {
		// MIGRATIONS
		if services().globals.database_version()? < 1 {
			for (roomserverid, _) in db.roomserverids.iter() {
				let mut parts = roomserverid.split(|&b| b == 0xFF);
				let room_id = parts.next().expect("split always returns one element");
				let Some(servername) = parts.next() else {
					error!("Migration: Invalid roomserverid in db.");
					continue;
				};
				let mut serverroomid = servername.to_vec();
				serverroomid.push(0xFF);
				serverroomid.extend_from_slice(room_id);

				db.serverroomids.insert(&serverroomid, &[])?;
			}

			services().globals.bump_database_version(1)?;

			warn!("Migration: 0 -> 1 finished");
		}

		if services().globals.database_version()? < 2 {
			// We accidentally inserted hashed versions of "" into the db instead of just ""
			for (userid, password) in db.userid_password.iter() {
				let salt = SaltString::generate(thread_rng());
				let empty_pass = services()
					.globals
					.argon
					.hash_password(b"", &salt)
					.expect("our own password to be properly hashed");
				let empty_hashed_password = services()
					.globals
					.argon
					.verify_password(&password, &empty_pass)
					.is_ok();

				if empty_hashed_password {
					db.userid_password.insert(&userid, b"")?;
				}
			}

			services().globals.bump_database_version(2)?;

			warn!("Migration: 1 -> 2 finished");
		}

		if services().globals.database_version()? < 3 {
			// Move media to filesystem
			for (key, content) in db.mediaid_file.iter() {
				if content.is_empty() {
					continue;
				}

				#[allow(deprecated)]
				let path = services().globals.get_media_file(&key);
				let mut file = fs::File::create(path)?;
				file.write_all(&content)?;
				db.mediaid_file.insert(&key, &[])?;
			}

			services().globals.bump_database_version(3)?;

			warn!("Migration: 2 -> 3 finished");
		}

		if services().globals.database_version()? < 4 {
			// Add federated users to services() as deactivated
			for our_user in services().users.iter() {
				let our_user = our_user?;
				if services().users.is_deactivated(&our_user)? {
					continue;
				}
				for room in services().rooms.state_cache.rooms_joined(&our_user) {
					for user in services().rooms.state_cache.room_members(&room?) {
						let user = user?;
						if user.server_name() != config.server_name {
							info!(?user, "Migration: creating user");
							services().users.create(&user, None)?;
						}
					}
				}
			}

			services().globals.bump_database_version(4)?;

			warn!("Migration: 3 -> 4 finished");
		}

		if services().globals.database_version()? < 5 {
			// Upgrade user data store
			for (roomuserdataid, _) in db.roomuserdataid_accountdata.iter() {
				let mut parts = roomuserdataid.split(|&b| b == 0xFF);
				let room_id = parts.next().unwrap();
				let user_id = parts.next().unwrap();
				let event_type = roomuserdataid.rsplit(|&b| b == 0xFF).next().unwrap();

				let mut key = room_id.to_vec();
				key.push(0xFF);
				key.extend_from_slice(user_id);
				key.push(0xFF);
				key.extend_from_slice(event_type);

				db.roomusertype_roomuserdataid
					.insert(&key, &roomuserdataid)?;
			}

			services().globals.bump_database_version(5)?;

			warn!("Migration: 4 -> 5 finished");
		}

		if services().globals.database_version()? < 6 {
			// Set room member count
			for (roomid, _) in db.roomid_shortstatehash.iter() {
				let string = utils::string_from_bytes(&roomid).unwrap();
				let room_id = <&RoomId>::try_from(string.as_str()).unwrap();
				services().rooms.state_cache.update_joined_count(room_id)?;
			}

			services().globals.bump_database_version(6)?;

			warn!("Migration: 5 -> 6 finished");
		}

		if services().globals.database_version()? < 7 {
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
						services()
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

				services().rooms.state_compressor.save_state_from_diff(
					current_sstatehash,
					Arc::new(statediffnew),
					Arc::new(statediffremoved),
					2, // every state change is 2 event changes on average
					states_parents,
				)?;

				/*
				let mut tmp = services().rooms.load_shortstatehash_info(&current_sstatehash)?;
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

			for (k, seventid) in db.db.open_tree("stateid_shorteventid")?.iter() {
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

					let event_id = db.shorteventid_eventid.get(&seventid).unwrap().unwrap();
					let string = utils::string_from_bytes(&event_id).unwrap();
					let event_id = <&EventId>::try_from(string.as_str()).unwrap();
					let pdu = services()
						.rooms
						.timeline
						.get_pdu(event_id)
						.unwrap()
						.unwrap();

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

			services().globals.bump_database_version(7)?;

			warn!("Migration: 6 -> 7 finished");
		}

		if services().globals.database_version()? < 8 {
			// Generate short room ids for all rooms
			for (room_id, _) in db.roomid_shortstatehash.iter() {
				let shortroomid = services().globals.next_count()?.to_be_bytes();
				db.roomid_shortroomid.insert(&room_id, &shortroomid)?;
				info!("Migration: 8");
			}
			// Update pduids db layout
			let mut batch = db.pduid_pdu.iter().filter_map(|(key, v)| {
				if !key.starts_with(b"!") {
					return None;
				}
				let mut parts = key.splitn(2, |&b| b == 0xFF);
				let room_id = parts.next().unwrap();
				let count = parts.next().unwrap();

				let short_room_id = db
					.roomid_shortroomid
					.get(room_id)
					.unwrap()
					.expect("shortroomid should exist");

				let mut new_key = short_room_id;
				new_key.extend_from_slice(count);

				Some((new_key, v))
			});

			db.pduid_pdu.insert_batch(&mut batch)?;

			let mut batch2 = db.eventid_pduid.iter().filter_map(|(k, value)| {
				if !value.starts_with(b"!") {
					return None;
				}
				let mut parts = value.splitn(2, |&b| b == 0xFF);
				let room_id = parts.next().unwrap();
				let count = parts.next().unwrap();

				let short_room_id = db
					.roomid_shortroomid
					.get(room_id)
					.unwrap()
					.expect("shortroomid should exist");

				let mut new_value = short_room_id;
				new_value.extend_from_slice(count);

				Some((k, new_value))
			});

			db.eventid_pduid.insert_batch(&mut batch2)?;

			services().globals.bump_database_version(8)?;

			warn!("Migration: 7 -> 8 finished");
		}

		if services().globals.database_version()? < 9 {
			// Update tokenids db layout
			let mut iter = db
				.tokenids
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

					let short_room_id = db
						.roomid_shortroomid
						.get(room_id)
						.unwrap()
						.expect("shortroomid should exist");
					let mut new_key = short_room_id;
					new_key.extend_from_slice(word);
					new_key.push(0xFF);
					new_key.extend_from_slice(pdu_id_count);
					Some((new_key, Vec::new()))
				})
				.peekable();

			while iter.peek().is_some() {
				db.tokenids.insert_batch(&mut iter.by_ref().take(1000))?;
				debug!("Inserted smaller batch");
			}

			info!("Deleting starts");

			let batch2: Vec<_> = db
				.tokenids
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
				db.tokenids.remove(&key)?;
			}

			services().globals.bump_database_version(9)?;

			warn!("Migration: 8 -> 9 finished");
		}

		if services().globals.database_version()? < 10 {
			// Add other direction for shortstatekeys
			for (statekey, shortstatekey) in db.statekey_shortstatekey.iter() {
				db.shortstatekey_statekey
					.insert(&shortstatekey, &statekey)?;
			}

			// Force E2EE device list updates so we can send them over federation
			for user_id in services().users.iter().filter_map(Result::ok) {
				services().users.mark_device_key_update(&user_id)?;
			}

			services().globals.bump_database_version(10)?;

			warn!("Migration: 9 -> 10 finished");
		}

		if services().globals.database_version()? < 11 {
			db.db
				.open_tree("userdevicesessionid_uiaarequest")?
				.clear()?;
			services().globals.bump_database_version(11)?;

			warn!("Migration: 10 -> 11 finished");
		}

		if services().globals.database_version()? < 12 {
			for username in services().users.list_local_users()? {
				let user = match UserId::parse_with_server_name(username.clone(), &config.server_name) {
					Ok(u) => u,
					Err(e) => {
						warn!("Invalid username {username}: {e}");
						continue;
					},
				};

				let raw_rules_list = services()
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

				services().account_data.update(
					None,
					&user,
					GlobalAccountDataEventType::PushRules.to_string().into(),
					&serde_json::to_value(account_data).expect("to json value always works"),
				)?;
			}

			services().globals.bump_database_version(12)?;

			warn!("Migration: 11 -> 12 finished");
		}

		// This migration can be reused as-is anytime the server-default rules are
		// updated.
		if services().globals.database_version()? < 13 {
			for username in services().users.list_local_users()? {
				let user = match UserId::parse_with_server_name(username.clone(), &config.server_name) {
					Ok(u) => u,
					Err(e) => {
						warn!("Invalid username {username}: {e}");
						continue;
					},
				};

				let raw_rules_list = services()
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

				services().account_data.update(
					None,
					&user,
					GlobalAccountDataEventType::PushRules.to_string().into(),
					&serde_json::to_value(account_data).expect("to json value always works"),
				)?;
			}

			services().globals.bump_database_version(13)?;

			warn!("Migration: 12 -> 13 finished");
		}

		#[cfg(feature = "sha256_media")]
		{
			if services().globals.database_version()? < 14 && cfg!(feature = "sha256_media") {
				warn!("sha256_media feature flag is enabled, migrating legacy base64 file names to sha256 file names");
				// Move old media files to new names
				for (key, _) in db.mediaid_file.iter() {
					let old_path = services().globals.get_media_file(&key);
					debug!("Old file path: {old_path:?}");
					let path = services().globals.get_media_file_new(&key);
					debug!("New file path: {path:?}");
					// move the file to the new location
					if old_path.exists() {
						tokio::fs::rename(&old_path, &path).await?;
					}
				}

				services().globals.bump_database_version(14)?;

				warn!("Migration: 13 -> 14 finished");
			}
		}

		assert_eq!(
			services().globals.database_version().unwrap(),
			latest_database_version,
			"Failed asserting local database version {} is equal to known latest conduwuit database version {}",
			services().globals.database_version().unwrap(),
			latest_database_version
		);

		{
			let patterns = &config.forbidden_usernames;
			if !patterns.is_empty() {
				for user_id in services()
					.users
					.iter()
					.filter_map(Result::ok)
					.filter(|user| !services().users.is_deactivated(user).unwrap_or(true))
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
			let patterns = &config.forbidden_alias_names;
			if !patterns.is_empty() {
				for address in services().rooms.metadata.iter_ids() {
					let room_id = address?;
					let room_aliases = services().rooms.alias.local_aliases_for_room(&room_id);
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
			"Loaded {} database with schema version {}",
			config.database_backend, latest_database_version
		);
	} else {
		services()
			.globals
			.bump_database_version(latest_database_version)?;

		// Create the admin room and server user on first run
		services().admin.create_admin_room().await?;

		warn!(
			"Created new {} database with version {}",
			config.database_backend, latest_database_version
		);
	}

	Ok(())
}
