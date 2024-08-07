use std::{collections::BTreeMap, sync::Arc};

use conduit::{utils, Error, Result};
use database::Map;
use ruma::{
	api::client::{
		backup::{BackupAlgorithm, KeyBackupData, RoomKeyBackup},
		error::ErrorKind,
	},
	serde::Raw,
	OwnedRoomId, RoomId, UserId,
};

use crate::{globals, Dep};

pub(super) struct Data {
	backupid_algorithm: Arc<Map>,
	backupid_etag: Arc<Map>,
	backupkeyid_backup: Arc<Map>,
	services: Services,
}

struct Services {
	globals: Dep<globals::Service>,
}

impl Data {
	pub(super) fn new(args: &crate::Args<'_>) -> Self {
		let db = &args.db;
		Self {
			backupid_algorithm: db["backupid_algorithm"].clone(),
			backupid_etag: db["backupid_etag"].clone(),
			backupkeyid_backup: db["backupkeyid_backup"].clone(),
			services: Services {
				globals: args.depend::<globals::Service>("globals"),
			},
		}
	}

	pub(super) fn create_backup(&self, user_id: &UserId, backup_metadata: &Raw<BackupAlgorithm>) -> Result<String> {
		let version = self.services.globals.next_count()?.to_string();

		let mut key = user_id.as_bytes().to_vec();
		key.push(0xFF);
		key.extend_from_slice(version.as_bytes());

		self.backupid_algorithm.insert(
			&key,
			&serde_json::to_vec(backup_metadata).expect("BackupAlgorithm::to_vec always works"),
		)?;
		self.backupid_etag
			.insert(&key, &self.services.globals.next_count()?.to_be_bytes())?;
		Ok(version)
	}

	pub(super) fn delete_backup(&self, user_id: &UserId, version: &str) -> Result<()> {
		let mut key = user_id.as_bytes().to_vec();
		key.push(0xFF);
		key.extend_from_slice(version.as_bytes());

		self.backupid_algorithm.remove(&key)?;
		self.backupid_etag.remove(&key)?;

		key.push(0xFF);

		for (outdated_key, _) in self.backupkeyid_backup.scan_prefix(key) {
			self.backupkeyid_backup.remove(&outdated_key)?;
		}

		Ok(())
	}

	pub(super) fn update_backup(
		&self, user_id: &UserId, version: &str, backup_metadata: &Raw<BackupAlgorithm>,
	) -> Result<String> {
		let mut key = user_id.as_bytes().to_vec();
		key.push(0xFF);
		key.extend_from_slice(version.as_bytes());

		if self.backupid_algorithm.get(&key)?.is_none() {
			return Err(Error::BadRequest(ErrorKind::NotFound, "Tried to update nonexistent backup."));
		}

		self.backupid_algorithm
			.insert(&key, backup_metadata.json().get().as_bytes())?;
		self.backupid_etag
			.insert(&key, &self.services.globals.next_count()?.to_be_bytes())?;
		Ok(version.to_owned())
	}

	pub(super) fn get_latest_backup_version(&self, user_id: &UserId) -> Result<Option<String>> {
		let mut prefix = user_id.as_bytes().to_vec();
		prefix.push(0xFF);
		let mut last_possible_key = prefix.clone();
		last_possible_key.extend_from_slice(&u64::MAX.to_be_bytes());

		self.backupid_algorithm
			.iter_from(&last_possible_key, true)
			.take_while(move |(k, _)| k.starts_with(&prefix))
			.next()
			.map(|(key, _)| {
				utils::string_from_bytes(
					key.rsplit(|&b| b == 0xFF)
						.next()
						.expect("rsplit always returns an element"),
				)
				.map_err(|_| Error::bad_database("backupid_algorithm key is invalid."))
			})
			.transpose()
	}

	pub(super) fn get_latest_backup(&self, user_id: &UserId) -> Result<Option<(String, Raw<BackupAlgorithm>)>> {
		let mut prefix = user_id.as_bytes().to_vec();
		prefix.push(0xFF);
		let mut last_possible_key = prefix.clone();
		last_possible_key.extend_from_slice(&u64::MAX.to_be_bytes());

		self.backupid_algorithm
			.iter_from(&last_possible_key, true)
			.take_while(move |(k, _)| k.starts_with(&prefix))
			.next()
			.map(|(key, value)| {
				let version = utils::string_from_bytes(
					key.rsplit(|&b| b == 0xFF)
						.next()
						.expect("rsplit always returns an element"),
				)
				.map_err(|_| Error::bad_database("backupid_algorithm key is invalid."))?;

				Ok((
					version,
					serde_json::from_slice(&value)
						.map_err(|_| Error::bad_database("Algorithm in backupid_algorithm is invalid."))?,
				))
			})
			.transpose()
	}

	pub(super) fn get_backup(&self, user_id: &UserId, version: &str) -> Result<Option<Raw<BackupAlgorithm>>> {
		let mut key = user_id.as_bytes().to_vec();
		key.push(0xFF);
		key.extend_from_slice(version.as_bytes());

		self.backupid_algorithm
			.get(&key)?
			.map_or(Ok(None), |bytes| {
				serde_json::from_slice(&bytes)
					.map_err(|_| Error::bad_database("Algorithm in backupid_algorithm is invalid."))
			})
	}

	pub(super) fn add_key(
		&self, user_id: &UserId, version: &str, room_id: &RoomId, session_id: &str, key_data: &Raw<KeyBackupData>,
	) -> Result<()> {
		let mut key = user_id.as_bytes().to_vec();
		key.push(0xFF);
		key.extend_from_slice(version.as_bytes());

		if self.backupid_algorithm.get(&key)?.is_none() {
			return Err(Error::BadRequest(ErrorKind::NotFound, "Tried to update nonexistent backup."));
		}

		self.backupid_etag
			.insert(&key, &self.services.globals.next_count()?.to_be_bytes())?;

		key.push(0xFF);
		key.extend_from_slice(room_id.as_bytes());
		key.push(0xFF);
		key.extend_from_slice(session_id.as_bytes());

		self.backupkeyid_backup
			.insert(&key, key_data.json().get().as_bytes())?;

		Ok(())
	}

	pub(super) fn count_keys(&self, user_id: &UserId, version: &str) -> Result<usize> {
		let mut prefix = user_id.as_bytes().to_vec();
		prefix.push(0xFF);
		prefix.extend_from_slice(version.as_bytes());

		Ok(self.backupkeyid_backup.scan_prefix(prefix).count())
	}

	pub(super) fn get_etag(&self, user_id: &UserId, version: &str) -> Result<String> {
		let mut key = user_id.as_bytes().to_vec();
		key.push(0xFF);
		key.extend_from_slice(version.as_bytes());

		Ok(utils::u64_from_bytes(
			&self
				.backupid_etag
				.get(&key)?
				.ok_or_else(|| Error::bad_database("Backup has no etag."))?,
		)
		.map_err(|_| Error::bad_database("etag in backupid_etag invalid."))?
		.to_string())
	}

	pub(super) fn get_all(&self, user_id: &UserId, version: &str) -> Result<BTreeMap<OwnedRoomId, RoomKeyBackup>> {
		let mut prefix = user_id.as_bytes().to_vec();
		prefix.push(0xFF);
		prefix.extend_from_slice(version.as_bytes());
		prefix.push(0xFF);

		let mut rooms = BTreeMap::<OwnedRoomId, RoomKeyBackup>::new();

		for result in self
			.backupkeyid_backup
			.scan_prefix(prefix)
			.map(|(key, value)| {
				let mut parts = key.rsplit(|&b| b == 0xFF);

				let session_id = utils::string_from_bytes(
					parts
						.next()
						.ok_or_else(|| Error::bad_database("backupkeyid_backup key is invalid."))?,
				)
				.map_err(|_| Error::bad_database("backupkeyid_backup session_id is invalid."))?;

				let room_id = RoomId::parse(
					utils::string_from_bytes(
						parts
							.next()
							.ok_or_else(|| Error::bad_database("backupkeyid_backup key is invalid."))?,
					)
					.map_err(|_| Error::bad_database("backupkeyid_backup room_id is invalid."))?,
				)
				.map_err(|_| Error::bad_database("backupkeyid_backup room_id is invalid room id."))?;

				let key_data = serde_json::from_slice(&value)
					.map_err(|_| Error::bad_database("KeyBackupData in backupkeyid_backup is invalid."))?;

				Ok::<_, Error>((room_id, session_id, key_data))
			}) {
			let (room_id, session_id, key_data) = result?;
			rooms
				.entry(room_id)
				.or_insert_with(|| RoomKeyBackup {
					sessions: BTreeMap::new(),
				})
				.sessions
				.insert(session_id, key_data);
		}

		Ok(rooms)
	}

	pub(super) fn get_room(
		&self, user_id: &UserId, version: &str, room_id: &RoomId,
	) -> Result<BTreeMap<String, Raw<KeyBackupData>>> {
		let mut prefix = user_id.as_bytes().to_vec();
		prefix.push(0xFF);
		prefix.extend_from_slice(version.as_bytes());
		prefix.push(0xFF);
		prefix.extend_from_slice(room_id.as_bytes());
		prefix.push(0xFF);

		Ok(self
			.backupkeyid_backup
			.scan_prefix(prefix)
			.map(|(key, value)| {
				let mut parts = key.rsplit(|&b| b == 0xFF);

				let session_id = utils::string_from_bytes(
					parts
						.next()
						.ok_or_else(|| Error::bad_database("backupkeyid_backup key is invalid."))?,
				)
				.map_err(|_| Error::bad_database("backupkeyid_backup session_id is invalid."))?;

				let key_data = serde_json::from_slice(&value)
					.map_err(|_| Error::bad_database("KeyBackupData in backupkeyid_backup is invalid."))?;

				Ok::<_, Error>((session_id, key_data))
			})
			.filter_map(Result::ok)
			.collect())
	}

	pub(super) fn get_session(
		&self, user_id: &UserId, version: &str, room_id: &RoomId, session_id: &str,
	) -> Result<Option<Raw<KeyBackupData>>> {
		let mut key = user_id.as_bytes().to_vec();
		key.push(0xFF);
		key.extend_from_slice(version.as_bytes());
		key.push(0xFF);
		key.extend_from_slice(room_id.as_bytes());
		key.push(0xFF);
		key.extend_from_slice(session_id.as_bytes());

		self.backupkeyid_backup
			.get(&key)?
			.map(|value| {
				serde_json::from_slice(&value)
					.map_err(|_| Error::bad_database("KeyBackupData in backupkeyid_backup is invalid."))
			})
			.transpose()
	}

	pub(super) fn delete_all_keys(&self, user_id: &UserId, version: &str) -> Result<()> {
		let mut key = user_id.as_bytes().to_vec();
		key.push(0xFF);
		key.extend_from_slice(version.as_bytes());
		key.push(0xFF);

		for (outdated_key, _) in self.backupkeyid_backup.scan_prefix(key) {
			self.backupkeyid_backup.remove(&outdated_key)?;
		}

		Ok(())
	}

	pub(super) fn delete_room_keys(&self, user_id: &UserId, version: &str, room_id: &RoomId) -> Result<()> {
		let mut key = user_id.as_bytes().to_vec();
		key.push(0xFF);
		key.extend_from_slice(version.as_bytes());
		key.push(0xFF);
		key.extend_from_slice(room_id.as_bytes());
		key.push(0xFF);

		for (outdated_key, _) in self.backupkeyid_backup.scan_prefix(key) {
			self.backupkeyid_backup.remove(&outdated_key)?;
		}

		Ok(())
	}

	pub(super) fn delete_room_key(
		&self, user_id: &UserId, version: &str, room_id: &RoomId, session_id: &str,
	) -> Result<()> {
		let mut key = user_id.as_bytes().to_vec();
		key.push(0xFF);
		key.extend_from_slice(version.as_bytes());
		key.push(0xFF);
		key.extend_from_slice(room_id.as_bytes());
		key.push(0xFF);
		key.extend_from_slice(session_id.as_bytes());

		for (outdated_key, _) in self.backupkeyid_backup.scan_prefix(key) {
			self.backupkeyid_backup.remove(&outdated_key)?;
		}

		Ok(())
	}
}
