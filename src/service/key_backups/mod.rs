use std::{collections::BTreeMap, sync::Arc};

use conduit::{
	err, implement, utils,
	utils::stream::{ReadyExt, TryIgnore},
	Err, Error, Result,
};
use database::{Deserialized, Ignore, Interfix, Map};
use futures::StreamExt;
use ruma::{
	api::client::backup::{BackupAlgorithm, KeyBackupData, RoomKeyBackup},
	serde::Raw,
	OwnedRoomId, RoomId, UserId,
};

use crate::{globals, Dep};

pub struct Service {
	db: Data,
	services: Services,
}

struct Data {
	backupid_algorithm: Arc<Map>,
	backupid_etag: Arc<Map>,
	backupkeyid_backup: Arc<Map>,
}

struct Services {
	globals: Dep<globals::Service>,
}

impl crate::Service for Service {
	fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
		Ok(Arc::new(Self {
			db: Data {
				backupid_algorithm: args.db["backupid_algorithm"].clone(),
				backupid_etag: args.db["backupid_etag"].clone(),
				backupkeyid_backup: args.db["backupkeyid_backup"].clone(),
			},
			services: Services {
				globals: args.depend::<globals::Service>("globals"),
			},
		}))
	}

	fn name(&self) -> &str { crate::service::make_name(std::module_path!()) }
}

#[implement(Service)]
pub fn create_backup(&self, user_id: &UserId, backup_metadata: &Raw<BackupAlgorithm>) -> Result<String> {
	let version = self.services.globals.next_count()?.to_string();

	let mut key = user_id.as_bytes().to_vec();
	key.push(0xFF);
	key.extend_from_slice(version.as_bytes());

	self.db.backupid_algorithm.insert(
		&key,
		&serde_json::to_vec(backup_metadata).expect("BackupAlgorithm::to_vec always works"),
	);

	self.db
		.backupid_etag
		.insert(&key, &self.services.globals.next_count()?.to_be_bytes());

	Ok(version)
}

#[implement(Service)]
pub async fn delete_backup(&self, user_id: &UserId, version: &str) {
	let mut key = user_id.as_bytes().to_vec();
	key.push(0xFF);
	key.extend_from_slice(version.as_bytes());

	self.db.backupid_algorithm.remove(&key);
	self.db.backupid_etag.remove(&key);

	let key = (user_id, version, Interfix);
	self.db
		.backupkeyid_backup
		.keys_raw_prefix(&key)
		.ignore_err()
		.ready_for_each(|outdated_key| self.db.backupkeyid_backup.remove(outdated_key))
		.await;
}

#[implement(Service)]
pub async fn update_backup(
	&self, user_id: &UserId, version: &str, backup_metadata: &Raw<BackupAlgorithm>,
) -> Result<String> {
	let key = (user_id, version);
	if self.db.backupid_algorithm.qry(&key).await.is_err() {
		return Err!(Request(NotFound("Tried to update nonexistent backup.")));
	}

	let mut key = user_id.as_bytes().to_vec();
	key.push(0xFF);
	key.extend_from_slice(version.as_bytes());

	self.db
		.backupid_algorithm
		.insert(&key, backup_metadata.json().get().as_bytes());
	self.db
		.backupid_etag
		.insert(&key, &self.services.globals.next_count()?.to_be_bytes());

	Ok(version.to_owned())
}

#[implement(Service)]
pub async fn get_latest_backup_version(&self, user_id: &UserId) -> Result<String> {
	let mut prefix = user_id.as_bytes().to_vec();
	prefix.push(0xFF);
	let mut last_possible_key = prefix.clone();
	last_possible_key.extend_from_slice(&u64::MAX.to_be_bytes());

	self.db
		.backupid_algorithm
		.rev_raw_keys_from(&last_possible_key)
		.ignore_err()
		.ready_take_while(move |key| key.starts_with(&prefix))
		.next()
		.await
		.ok_or_else(|| err!(Request(NotFound("No backup versions found"))))
		.and_then(|key| {
			utils::string_from_bytes(
				key.rsplit(|&b| b == 0xFF)
					.next()
					.expect("rsplit always returns an element"),
			)
			.map_err(|_| Error::bad_database("backupid_algorithm key is invalid."))
		})
}

#[implement(Service)]
pub async fn get_latest_backup(&self, user_id: &UserId) -> Result<(String, Raw<BackupAlgorithm>)> {
	let mut prefix = user_id.as_bytes().to_vec();
	prefix.push(0xFF);
	let mut last_possible_key = prefix.clone();
	last_possible_key.extend_from_slice(&u64::MAX.to_be_bytes());

	self.db
		.backupid_algorithm
		.rev_raw_stream_from(&last_possible_key)
		.ignore_err()
		.ready_take_while(move |(key, _)| key.starts_with(&prefix))
		.next()
		.await
		.ok_or_else(|| err!(Request(NotFound("No backup found"))))
		.and_then(|(key, val)| {
			let version = utils::string_from_bytes(
				key.rsplit(|&b| b == 0xFF)
					.next()
					.expect("rsplit always returns an element"),
			)
			.map_err(|_| Error::bad_database("backupid_algorithm key is invalid."))?;

			let algorithm = serde_json::from_slice(val)
				.map_err(|_| Error::bad_database("Algorithm in backupid_algorithm is invalid."))?;

			Ok((version, algorithm))
		})
}

#[implement(Service)]
pub async fn get_backup(&self, user_id: &UserId, version: &str) -> Result<Raw<BackupAlgorithm>> {
	let key = (user_id, version);
	self.db.backupid_algorithm.qry(&key).await.deserialized()
}

#[implement(Service)]
pub async fn add_key(
	&self, user_id: &UserId, version: &str, room_id: &RoomId, session_id: &str, key_data: &Raw<KeyBackupData>,
) -> Result<()> {
	let key = (user_id, version);
	if self.db.backupid_algorithm.qry(&key).await.is_err() {
		return Err!(Request(NotFound("Tried to update nonexistent backup.")));
	}

	let mut key = user_id.as_bytes().to_vec();
	key.push(0xFF);
	key.extend_from_slice(version.as_bytes());

	self.db
		.backupid_etag
		.insert(&key, &self.services.globals.next_count()?.to_be_bytes());

	key.push(0xFF);
	key.extend_from_slice(room_id.as_bytes());
	key.push(0xFF);
	key.extend_from_slice(session_id.as_bytes());

	self.db
		.backupkeyid_backup
		.insert(&key, key_data.json().get().as_bytes());

	Ok(())
}

#[implement(Service)]
pub async fn count_keys(&self, user_id: &UserId, version: &str) -> usize {
	let prefix = (user_id, version);
	self.db
		.backupkeyid_backup
		.keys_raw_prefix(&prefix)
		.count()
		.await
}

#[implement(Service)]
pub async fn get_etag(&self, user_id: &UserId, version: &str) -> String {
	let key = (user_id, version);
	self.db
		.backupid_etag
		.qry(&key)
		.await
		.deserialized::<u64>()
		.as_ref()
		.map(ToString::to_string)
		.expect("Backup has no etag.")
}

#[implement(Service)]
pub async fn get_all(&self, user_id: &UserId, version: &str) -> BTreeMap<OwnedRoomId, RoomKeyBackup> {
	type KeyVal<'a> = ((Ignore, Ignore, &'a RoomId, &'a str), &'a [u8]);

	let mut rooms = BTreeMap::<OwnedRoomId, RoomKeyBackup>::new();
	let default = || RoomKeyBackup {
		sessions: BTreeMap::new(),
	};

	let prefix = (user_id, version, Interfix);
	self.db
		.backupkeyid_backup
		.stream_prefix(&prefix)
		.ignore_err()
		.ready_for_each(|((_, _, room_id, session_id), value): KeyVal<'_>| {
			let key_data = serde_json::from_slice(value).expect("Invalid KeyBackupData JSON");
			rooms
				.entry(room_id.into())
				.or_insert_with(default)
				.sessions
				.insert(session_id.into(), key_data);
		})
		.await;

	rooms
}

#[implement(Service)]
pub async fn get_room(
	&self, user_id: &UserId, version: &str, room_id: &RoomId,
) -> BTreeMap<String, Raw<KeyBackupData>> {
	type KeyVal<'a> = ((Ignore, Ignore, Ignore, &'a str), &'a [u8]);

	let prefix = (user_id, version, room_id, Interfix);
	self.db
		.backupkeyid_backup
		.stream_prefix(&prefix)
		.ignore_err()
		.map(|((.., session_id), value): KeyVal<'_>| {
			let session_id = session_id.to_owned();
			let key_backup_data = serde_json::from_slice(value).expect("Invalid KeyBackupData JSON");
			(session_id, key_backup_data)
		})
		.collect()
		.await
}

#[implement(Service)]
pub async fn get_session(
	&self, user_id: &UserId, version: &str, room_id: &RoomId, session_id: &str,
) -> Result<Raw<KeyBackupData>> {
	let key = (user_id, version, room_id, session_id);

	self.db.backupkeyid_backup.qry(&key).await.deserialized()
}

#[implement(Service)]
pub async fn delete_all_keys(&self, user_id: &UserId, version: &str) {
	let key = (user_id, version, Interfix);
	self.db
		.backupkeyid_backup
		.keys_raw_prefix(&key)
		.ignore_err()
		.ready_for_each(|outdated_key| self.db.backupkeyid_backup.remove(outdated_key))
		.await;
}

#[implement(Service)]
pub async fn delete_room_keys(&self, user_id: &UserId, version: &str, room_id: &RoomId) {
	let key = (user_id, version, room_id, Interfix);
	self.db
		.backupkeyid_backup
		.keys_raw_prefix(&key)
		.ignore_err()
		.ready_for_each(|outdated_key| self.db.backupkeyid_backup.remove(outdated_key))
		.await;
}

#[implement(Service)]
pub async fn delete_room_key(&self, user_id: &UserId, version: &str, room_id: &RoomId, session_id: &str) {
	let key = (user_id, version, room_id, session_id);
	self.db
		.backupkeyid_backup
		.keys_raw_prefix(&key)
		.ignore_err()
		.ready_for_each(|outdated_key| self.db.backupkeyid_backup.remove(outdated_key))
		.await;
}
