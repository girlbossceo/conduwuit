use std::{collections::BTreeMap, mem::size_of, sync::Arc};

use conduit::{debug_info, err, utils, warn, Err, Error, Result, Server};
use database::Map;
use ruma::{
	api::client::{device::Device, error::ErrorKind, filter::FilterDefinition},
	encryption::{CrossSigningKey, DeviceKeys, OneTimeKey},
	events::{AnyToDeviceEvent, StateEventType},
	serde::Raw,
	uint, DeviceId, DeviceKeyAlgorithm, DeviceKeyId, MilliSecondsSinceUnixEpoch, OwnedDeviceId, OwnedDeviceKeyId,
	OwnedMxcUri, OwnedUserId, UInt, UserId,
};

use crate::{globals, rooms, users::clean_signatures, Dep};

pub struct Data {
	keychangeid_userid: Arc<Map>,
	keyid_key: Arc<Map>,
	onetimekeyid_onetimekeys: Arc<Map>,
	openidtoken_expiresatuserid: Arc<Map>,
	todeviceid_events: Arc<Map>,
	token_userdeviceid: Arc<Map>,
	userdeviceid_metadata: Arc<Map>,
	userdeviceid_token: Arc<Map>,
	userfilterid_filter: Arc<Map>,
	userid_avatarurl: Arc<Map>,
	userid_blurhash: Arc<Map>,
	userid_devicelistversion: Arc<Map>,
	userid_displayname: Arc<Map>,
	userid_lastonetimekeyupdate: Arc<Map>,
	userid_masterkeyid: Arc<Map>,
	userid_password: Arc<Map>,
	userid_selfsigningkeyid: Arc<Map>,
	userid_usersigningkeyid: Arc<Map>,
	services: Services,
}

struct Services {
	server: Arc<Server>,
	globals: Dep<globals::Service>,
	state_cache: Dep<rooms::state_cache::Service>,
	state_accessor: Dep<rooms::state_accessor::Service>,
}

impl Data {
	pub(super) fn new(args: &crate::Args<'_>) -> Self {
		let db = &args.db;
		Self {
			keychangeid_userid: db["keychangeid_userid"].clone(),
			keyid_key: db["keyid_key"].clone(),
			onetimekeyid_onetimekeys: db["onetimekeyid_onetimekeys"].clone(),
			openidtoken_expiresatuserid: db["openidtoken_expiresatuserid"].clone(),
			todeviceid_events: db["todeviceid_events"].clone(),
			token_userdeviceid: db["token_userdeviceid"].clone(),
			userdeviceid_metadata: db["userdeviceid_metadata"].clone(),
			userdeviceid_token: db["userdeviceid_token"].clone(),
			userfilterid_filter: db["userfilterid_filter"].clone(),
			userid_avatarurl: db["userid_avatarurl"].clone(),
			userid_blurhash: db["userid_blurhash"].clone(),
			userid_devicelistversion: db["userid_devicelistversion"].clone(),
			userid_displayname: db["userid_displayname"].clone(),
			userid_lastonetimekeyupdate: db["userid_lastonetimekeyupdate"].clone(),
			userid_masterkeyid: db["userid_masterkeyid"].clone(),
			userid_password: db["userid_password"].clone(),
			userid_selfsigningkeyid: db["userid_selfsigningkeyid"].clone(),
			userid_usersigningkeyid: db["userid_usersigningkeyid"].clone(),
			services: Services {
				server: args.server.clone(),
				globals: args.depend::<globals::Service>("globals"),
				state_cache: args.depend::<rooms::state_cache::Service>("rooms::state_cache"),
				state_accessor: args.depend::<rooms::state_accessor::Service>("rooms::state_accessor"),
			},
		}
	}

	/// Check if a user has an account on this homeserver.
	#[inline]
	pub(super) fn exists(&self, user_id: &UserId) -> Result<bool> {
		Ok(self.userid_password.get(user_id.as_bytes())?.is_some())
	}

	/// Check if account is deactivated
	pub(super) fn is_deactivated(&self, user_id: &UserId) -> Result<bool> {
		Ok(self
			.userid_password
			.get(user_id.as_bytes())?
			.ok_or(Error::BadRequest(ErrorKind::InvalidParam, "User does not exist."))?
			.is_empty())
	}

	/// Returns the number of users registered on this server.
	#[inline]
	pub(super) fn count(&self) -> Result<usize> { Ok(self.userid_password.iter().count()) }

	/// Find out which user an access token belongs to.
	pub(super) fn find_from_token(&self, token: &str) -> Result<Option<(OwnedUserId, String)>> {
		self.token_userdeviceid
			.get(token.as_bytes())?
			.map_or(Ok(None), |bytes| {
				let mut parts = bytes.split(|&b| b == 0xFF);
				let user_bytes = parts
					.next()
					.ok_or_else(|| err!(Database("User ID in token_userdeviceid is invalid.")))?;
				let device_bytes = parts
					.next()
					.ok_or_else(|| err!(Database("Device ID in token_userdeviceid is invalid.")))?;

				Ok(Some((
					UserId::parse(
						utils::string_from_bytes(user_bytes)
							.map_err(|e| err!(Database("User ID in token_userdeviceid is invalid unicode. {e}")))?,
					)
					.map_err(|e| err!(Database("User ID in token_userdeviceid is invalid. {e}")))?,
					utils::string_from_bytes(device_bytes)
						.map_err(|e| err!(Database("Device ID in token_userdeviceid is invalid. {e}")))?,
				)))
			})
	}

	/// Returns an iterator over all users on this homeserver.
	pub fn iter<'a>(&'a self) -> Box<dyn Iterator<Item = Result<OwnedUserId>> + 'a> {
		Box::new(self.userid_password.iter().map(|(bytes, _)| {
			UserId::parse(
				utils::string_from_bytes(&bytes)
					.map_err(|e| err!(Database("User ID in userid_password is invalid unicode. {e}")))?,
			)
			.map_err(|e| err!(Database("User ID in userid_password is invalid. {e}")))
		}))
	}

	/// Returns a list of local users as list of usernames.
	///
	/// A user account is considered `local` if the length of it's password is
	/// greater then zero.
	pub(super) fn list_local_users(&self) -> Result<Vec<String>> {
		let users: Vec<String> = self
			.userid_password
			.iter()
			.filter_map(|(username, pw)| get_username_with_valid_password(&username, &pw))
			.collect();
		Ok(users)
	}

	/// Returns the password hash for the given user.
	pub(super) fn password_hash(&self, user_id: &UserId) -> Result<Option<String>> {
		self.userid_password
			.get(user_id.as_bytes())?
			.map_or(Ok(None), |bytes| {
				Ok(Some(utils::string_from_bytes(&bytes).map_err(|_| {
					Error::bad_database("Password hash in db is not valid string.")
				})?))
			})
	}

	/// Hash and set the user's password to the Argon2 hash
	pub(super) fn set_password(&self, user_id: &UserId, password: Option<&str>) -> Result<()> {
		if let Some(password) = password {
			if let Ok(hash) = utils::hash::password(password) {
				self.userid_password
					.insert(user_id.as_bytes(), hash.as_bytes())?;
				Ok(())
			} else {
				Err(Error::BadRequest(
					ErrorKind::InvalidParam,
					"Password does not meet the requirements.",
				))
			}
		} else {
			self.userid_password.insert(user_id.as_bytes(), b"")?;
			Ok(())
		}
	}

	/// Returns the displayname of a user on this homeserver.
	pub(super) fn displayname(&self, user_id: &UserId) -> Result<Option<String>> {
		self.userid_displayname
			.get(user_id.as_bytes())?
			.map_or(Ok(None), |bytes| {
				Ok(Some(
					utils::string_from_bytes(&bytes)
						.map_err(|e| err!(Database("Displayname in db is invalid. {e}")))?,
				))
			})
	}

	/// Sets a new displayname or removes it if displayname is None. You still
	/// need to nofify all rooms of this change.
	pub(super) fn set_displayname(&self, user_id: &UserId, displayname: Option<String>) -> Result<()> {
		if let Some(displayname) = displayname {
			self.userid_displayname
				.insert(user_id.as_bytes(), displayname.as_bytes())?;
		} else {
			self.userid_displayname.remove(user_id.as_bytes())?;
		}

		Ok(())
	}

	/// Get the `avatar_url` of a user.
	pub(super) fn avatar_url(&self, user_id: &UserId) -> Result<Option<OwnedMxcUri>> {
		self.userid_avatarurl
			.get(user_id.as_bytes())?
			.map(|bytes| {
				let s_bytes = utils::string_from_bytes(&bytes)
					.map_err(|e| err!(Database(warn!("Avatar URL in db is invalid: {e}"))))?;
				let mxc_uri: OwnedMxcUri = s_bytes.into();
				Ok(mxc_uri)
			})
			.transpose()
	}

	/// Sets a new avatar_url or removes it if avatar_url is None.
	pub(super) fn set_avatar_url(&self, user_id: &UserId, avatar_url: Option<OwnedMxcUri>) -> Result<()> {
		if let Some(avatar_url) = avatar_url {
			self.userid_avatarurl
				.insert(user_id.as_bytes(), avatar_url.to_string().as_bytes())?;
		} else {
			self.userid_avatarurl.remove(user_id.as_bytes())?;
		}

		Ok(())
	}

	/// Get the blurhash of a user.
	pub(super) fn blurhash(&self, user_id: &UserId) -> Result<Option<String>> {
		self.userid_blurhash
			.get(user_id.as_bytes())?
			.map(|bytes| {
				utils::string_from_bytes(&bytes).map_err(|e| err!(Database("Avatar URL in db is invalid. {e}")))
			})
			.transpose()
	}

	/// Sets a new avatar_url or removes it if avatar_url is None.
	pub(super) fn set_blurhash(&self, user_id: &UserId, blurhash: Option<String>) -> Result<()> {
		if let Some(blurhash) = blurhash {
			self.userid_blurhash
				.insert(user_id.as_bytes(), blurhash.as_bytes())?;
		} else {
			self.userid_blurhash.remove(user_id.as_bytes())?;
		}

		Ok(())
	}

	/// Adds a new device to a user.
	pub(super) fn create_device(
		&self, user_id: &UserId, device_id: &DeviceId, token: &str, initial_device_display_name: Option<String>,
		client_ip: Option<String>,
	) -> Result<()> {
		// This method should never be called for nonexistent users. We shouldn't assert
		// though...
		if !self.exists(user_id)? {
			warn!("Called create_device for non-existent user {} in database", user_id);
			return Err(Error::BadRequest(ErrorKind::InvalidParam, "User does not exist."));
		}

		let mut userdeviceid = user_id.as_bytes().to_vec();
		userdeviceid.push(0xFF);
		userdeviceid.extend_from_slice(device_id.as_bytes());

		self.userid_devicelistversion
			.increment(user_id.as_bytes())?;

		self.userdeviceid_metadata.insert(
			&userdeviceid,
			&serde_json::to_vec(&Device {
				device_id: device_id.into(),
				display_name: initial_device_display_name,
				last_seen_ip: client_ip,
				last_seen_ts: Some(MilliSecondsSinceUnixEpoch::now()),
			})
			.expect("Device::to_string never fails."),
		)?;

		self.set_token(user_id, device_id, token)?;

		Ok(())
	}

	/// Removes a device from a user.
	pub(super) fn remove_device(&self, user_id: &UserId, device_id: &DeviceId) -> Result<()> {
		let mut userdeviceid = user_id.as_bytes().to_vec();
		userdeviceid.push(0xFF);
		userdeviceid.extend_from_slice(device_id.as_bytes());

		// Remove tokens
		if let Some(old_token) = self.userdeviceid_token.get(&userdeviceid)? {
			self.userdeviceid_token.remove(&userdeviceid)?;
			self.token_userdeviceid.remove(&old_token)?;
		}

		// Remove todevice events
		let mut prefix = userdeviceid.clone();
		prefix.push(0xFF);

		for (key, _) in self.todeviceid_events.scan_prefix(prefix) {
			self.todeviceid_events.remove(&key)?;
		}

		// TODO: Remove onetimekeys

		self.userid_devicelistversion
			.increment(user_id.as_bytes())?;

		self.userdeviceid_metadata.remove(&userdeviceid)?;

		Ok(())
	}

	/// Returns an iterator over all device ids of this user.
	pub(super) fn all_device_ids<'a>(
		&'a self, user_id: &UserId,
	) -> Box<dyn Iterator<Item = Result<OwnedDeviceId>> + 'a> {
		let mut prefix = user_id.as_bytes().to_vec();
		prefix.push(0xFF);
		// All devices have metadata
		Box::new(
			self.userdeviceid_metadata
				.scan_prefix(prefix)
				.map(|(bytes, _)| {
					Ok(utils::string_from_bytes(
						bytes
							.rsplit(|&b| b == 0xFF)
							.next()
							.ok_or_else(|| err!(Database("UserDevice ID in db is invalid.")))?,
					)
					.map_err(|e| err!(Database("Device ID in userdeviceid_metadata is invalid. {e}")))?
					.into())
				}),
		)
	}

	/// Replaces the access token of one device.
	pub(super) fn set_token(&self, user_id: &UserId, device_id: &DeviceId, token: &str) -> Result<()> {
		let mut userdeviceid = user_id.as_bytes().to_vec();
		userdeviceid.push(0xFF);
		userdeviceid.extend_from_slice(device_id.as_bytes());

		// should not be None, but we shouldn't assert either lol...
		if self.userdeviceid_metadata.get(&userdeviceid)?.is_none() {
			return Err!(Database(error!(
				"User {user_id:?} does not exist or device ID {device_id:?} has no metadata."
			)));
		}

		// Remove old token
		if let Some(old_token) = self.userdeviceid_token.get(&userdeviceid)? {
			self.token_userdeviceid.remove(&old_token)?;
			// It will be removed from userdeviceid_token by the insert later
		}

		// Assign token to user device combination
		self.userdeviceid_token
			.insert(&userdeviceid, token.as_bytes())?;
		self.token_userdeviceid
			.insert(token.as_bytes(), &userdeviceid)?;

		Ok(())
	}

	pub(super) fn add_one_time_key(
		&self, user_id: &UserId, device_id: &DeviceId, one_time_key_key: &DeviceKeyId,
		one_time_key_value: &Raw<OneTimeKey>,
	) -> Result<()> {
		let mut key = user_id.as_bytes().to_vec();
		key.push(0xFF);
		key.extend_from_slice(device_id.as_bytes());

		// All devices have metadata
		// Only existing devices should be able to call this, but we shouldn't assert
		// either...
		if self.userdeviceid_metadata.get(&key)?.is_none() {
			return Err!(Database(error!(
				"User {user_id:?} does not exist or device ID {device_id:?} has no metadata."
			)));
		}

		key.push(0xFF);
		// TODO: Use DeviceKeyId::to_string when it's available (and update everything,
		// because there are no wrapping quotation marks anymore)
		key.extend_from_slice(
			serde_json::to_string(one_time_key_key)
				.expect("DeviceKeyId::to_string always works")
				.as_bytes(),
		);

		self.onetimekeyid_onetimekeys.insert(
			&key,
			&serde_json::to_vec(&one_time_key_value).expect("OneTimeKey::to_vec always works"),
		)?;

		self.userid_lastonetimekeyupdate
			.insert(user_id.as_bytes(), &self.services.globals.next_count()?.to_be_bytes())?;

		Ok(())
	}

	pub(super) fn last_one_time_keys_update(&self, user_id: &UserId) -> Result<u64> {
		self.userid_lastonetimekeyupdate
			.get(user_id.as_bytes())?
			.map_or(Ok(0), |bytes| {
				utils::u64_from_bytes(&bytes)
					.map_err(|e| err!(Database("Count in roomid_lastroomactiveupdate is invalid. {e}")))
			})
	}

	pub(super) fn take_one_time_key(
		&self, user_id: &UserId, device_id: &DeviceId, key_algorithm: &DeviceKeyAlgorithm,
	) -> Result<Option<(OwnedDeviceKeyId, Raw<OneTimeKey>)>> {
		let mut prefix = user_id.as_bytes().to_vec();
		prefix.push(0xFF);
		prefix.extend_from_slice(device_id.as_bytes());
		prefix.push(0xFF);
		prefix.push(b'"'); // Annoying quotation mark
		prefix.extend_from_slice(key_algorithm.as_ref().as_bytes());
		prefix.push(b':');

		self.userid_lastonetimekeyupdate
			.insert(user_id.as_bytes(), &self.services.globals.next_count()?.to_be_bytes())?;

		self.onetimekeyid_onetimekeys
			.scan_prefix(prefix)
			.next()
			.map(|(key, value)| {
				self.onetimekeyid_onetimekeys.remove(&key)?;

				Ok((
					serde_json::from_slice(
						key.rsplit(|&b| b == 0xFF)
							.next()
							.ok_or_else(|| err!(Database("OneTimeKeyId in db is invalid.")))?,
					)
					.map_err(|e| err!(Database("OneTimeKeyId in db is invalid. {e}")))?,
					serde_json::from_slice(&value).map_err(|e| err!(Database("OneTimeKeys in db are invalid. {e}")))?,
				))
			})
			.transpose()
	}

	pub(super) fn count_one_time_keys(
		&self, user_id: &UserId, device_id: &DeviceId,
	) -> Result<BTreeMap<DeviceKeyAlgorithm, UInt>> {
		let mut userdeviceid = user_id.as_bytes().to_vec();
		userdeviceid.push(0xFF);
		userdeviceid.extend_from_slice(device_id.as_bytes());

		let mut counts = BTreeMap::new();

		for algorithm in self
			.onetimekeyid_onetimekeys
			.scan_prefix(userdeviceid)
			.map(|(bytes, _)| {
				Ok::<_, Error>(
					serde_json::from_slice::<OwnedDeviceKeyId>(
						bytes
							.rsplit(|&b| b == 0xFF)
							.next()
							.ok_or_else(|| err!(Database("OneTimeKey ID in db is invalid.")))?,
					)
					.map_err(|e| err!(Database("DeviceKeyId in db is invalid. {e}")))?
					.algorithm(),
				)
			}) {
			let count: &mut UInt = counts.entry(algorithm?).or_default();
			*count = count.saturating_add(uint!(1));
		}

		Ok(counts)
	}

	pub(super) fn add_device_keys(
		&self, user_id: &UserId, device_id: &DeviceId, device_keys: &Raw<DeviceKeys>,
	) -> Result<()> {
		let mut userdeviceid = user_id.as_bytes().to_vec();
		userdeviceid.push(0xFF);
		userdeviceid.extend_from_slice(device_id.as_bytes());

		self.keyid_key.insert(
			&userdeviceid,
			&serde_json::to_vec(&device_keys).expect("DeviceKeys::to_vec always works"),
		)?;

		self.mark_device_key_update(user_id)?;

		Ok(())
	}

	pub(super) fn add_cross_signing_keys(
		&self, user_id: &UserId, master_key: &Raw<CrossSigningKey>, self_signing_key: &Option<Raw<CrossSigningKey>>,
		user_signing_key: &Option<Raw<CrossSigningKey>>, notify: bool,
	) -> Result<()> {
		// TODO: Check signatures
		let mut prefix = user_id.as_bytes().to_vec();
		prefix.push(0xFF);

		let (master_key_key, _) = Self::parse_master_key(user_id, master_key)?;

		self.keyid_key
			.insert(&master_key_key, master_key.json().get().as_bytes())?;

		self.userid_masterkeyid
			.insert(user_id.as_bytes(), &master_key_key)?;

		// Self-signing key
		if let Some(self_signing_key) = self_signing_key {
			let mut self_signing_key_ids = self_signing_key
				.deserialize()
				.map_err(|_| Error::BadRequest(ErrorKind::InvalidParam, "Invalid self signing key"))?
				.keys
				.into_values();

			let self_signing_key_id = self_signing_key_ids
				.next()
				.ok_or(Error::BadRequest(ErrorKind::InvalidParam, "Self signing key contained no key."))?;

			if self_signing_key_ids.next().is_some() {
				return Err(Error::BadRequest(
					ErrorKind::InvalidParam,
					"Self signing key contained more than one key.",
				));
			}

			let mut self_signing_key_key = prefix.clone();
			self_signing_key_key.extend_from_slice(self_signing_key_id.as_bytes());

			self.keyid_key
				.insert(&self_signing_key_key, self_signing_key.json().get().as_bytes())?;

			self.userid_selfsigningkeyid
				.insert(user_id.as_bytes(), &self_signing_key_key)?;
		}

		// User-signing key
		if let Some(user_signing_key) = user_signing_key {
			let mut user_signing_key_ids = user_signing_key
				.deserialize()
				.map_err(|_| Error::BadRequest(ErrorKind::InvalidParam, "Invalid user signing key"))?
				.keys
				.into_values();

			let user_signing_key_id = user_signing_key_ids
				.next()
				.ok_or(Error::BadRequest(ErrorKind::InvalidParam, "User signing key contained no key."))?;

			if user_signing_key_ids.next().is_some() {
				return Err(Error::BadRequest(
					ErrorKind::InvalidParam,
					"User signing key contained more than one key.",
				));
			}

			let mut user_signing_key_key = prefix;
			user_signing_key_key.extend_from_slice(user_signing_key_id.as_bytes());

			self.keyid_key
				.insert(&user_signing_key_key, user_signing_key.json().get().as_bytes())?;

			self.userid_usersigningkeyid
				.insert(user_id.as_bytes(), &user_signing_key_key)?;
		}

		if notify {
			self.mark_device_key_update(user_id)?;
		}

		Ok(())
	}

	pub(super) fn sign_key(
		&self, target_id: &UserId, key_id: &str, signature: (String, String), sender_id: &UserId,
	) -> Result<()> {
		let mut key = target_id.as_bytes().to_vec();
		key.push(0xFF);
		key.extend_from_slice(key_id.as_bytes());

		let mut cross_signing_key: serde_json::Value = serde_json::from_slice(
			&self
				.keyid_key
				.get(&key)?
				.ok_or(Error::BadRequest(ErrorKind::InvalidParam, "Tried to sign nonexistent key."))?,
		)
		.map_err(|e| err!(Database("key in keyid_key is invalid. {e}")))?;

		let signatures = cross_signing_key
			.get_mut("signatures")
			.ok_or_else(|| err!(Database("key in keyid_key has no signatures field.")))?
			.as_object_mut()
			.ok_or_else(|| err!(Database("key in keyid_key has invalid signatures field.")))?
			.entry(sender_id.to_string())
			.or_insert_with(|| serde_json::Map::new().into());

		signatures
			.as_object_mut()
			.ok_or_else(|| err!(Database("signatures in keyid_key for a user is invalid.")))?
			.insert(signature.0, signature.1.into());

		self.keyid_key.insert(
			&key,
			&serde_json::to_vec(&cross_signing_key).expect("CrossSigningKey::to_vec always works"),
		)?;

		self.mark_device_key_update(target_id)?;

		Ok(())
	}

	pub(super) fn keys_changed<'a>(
		&'a self, user_or_room_id: &str, from: u64, to: Option<u64>,
	) -> Box<dyn Iterator<Item = Result<OwnedUserId>> + 'a> {
		let mut prefix = user_or_room_id.as_bytes().to_vec();
		prefix.push(0xFF);

		let mut start = prefix.clone();
		start.extend_from_slice(&(from.saturating_add(1)).to_be_bytes());

		let to = to.unwrap_or(u64::MAX);

		Box::new(
			self.keychangeid_userid
				.iter_from(&start, false)
				.take_while(move |(k, _)| {
					k.starts_with(&prefix)
						&& if let Some(current) = k.splitn(2, |&b| b == 0xFF).nth(1) {
							if let Ok(c) = utils::u64_from_bytes(current) {
								c <= to
							} else {
								warn!("BadDatabase: Could not parse keychangeid_userid bytes");
								false
							}
						} else {
							warn!("BadDatabase: Could not parse keychangeid_userid");
							false
						}
				})
				.map(|(_, bytes)| {
					UserId::parse(
						utils::string_from_bytes(&bytes).map_err(|_| {
							Error::bad_database("User ID in devicekeychangeid_userid is invalid unicode.")
						})?,
					)
					.map_err(|e| err!(Database("User ID in devicekeychangeid_userid is invalid. {e}")))
				}),
		)
	}

	pub(super) fn mark_device_key_update(&self, user_id: &UserId) -> Result<()> {
		let count = self.services.globals.next_count()?.to_be_bytes();
		for room_id in self
			.services
			.state_cache
			.rooms_joined(user_id)
			.filter_map(Result::ok)
		{
			// Don't send key updates to unencrypted rooms
			if self
				.services
				.state_accessor
				.room_state_get(&room_id, &StateEventType::RoomEncryption, "")?
				.is_none()
			{
				continue;
			}

			let mut key = room_id.as_bytes().to_vec();
			key.push(0xFF);
			key.extend_from_slice(&count);

			self.keychangeid_userid.insert(&key, user_id.as_bytes())?;
		}

		let mut key = user_id.as_bytes().to_vec();
		key.push(0xFF);
		key.extend_from_slice(&count);
		self.keychangeid_userid.insert(&key, user_id.as_bytes())?;

		Ok(())
	}

	pub(super) fn get_device_keys(&self, user_id: &UserId, device_id: &DeviceId) -> Result<Option<Raw<DeviceKeys>>> {
		let mut key = user_id.as_bytes().to_vec();
		key.push(0xFF);
		key.extend_from_slice(device_id.as_bytes());

		self.keyid_key.get(&key)?.map_or(Ok(None), |bytes| {
			Ok(Some(
				serde_json::from_slice(&bytes).map_err(|e| err!(Database("DeviceKeys in db are invalid. {e}")))?,
			))
		})
	}

	pub(super) fn parse_master_key(
		user_id: &UserId, master_key: &Raw<CrossSigningKey>,
	) -> Result<(Vec<u8>, CrossSigningKey)> {
		let mut prefix = user_id.as_bytes().to_vec();
		prefix.push(0xFF);

		let master_key = master_key
			.deserialize()
			.map_err(|_| Error::BadRequest(ErrorKind::InvalidParam, "Invalid master key"))?;
		let mut master_key_ids = master_key.keys.values();
		let master_key_id = master_key_ids
			.next()
			.ok_or(Error::BadRequest(ErrorKind::InvalidParam, "Master key contained no key."))?;
		if master_key_ids.next().is_some() {
			return Err(Error::BadRequest(
				ErrorKind::InvalidParam,
				"Master key contained more than one key.",
			));
		}
		let mut master_key_key = prefix.clone();
		master_key_key.extend_from_slice(master_key_id.as_bytes());
		Ok((master_key_key, master_key))
	}

	pub(super) fn get_key(
		&self, key: &[u8], sender_user: Option<&UserId>, user_id: &UserId, allowed_signatures: &dyn Fn(&UserId) -> bool,
	) -> Result<Option<Raw<CrossSigningKey>>> {
		self.keyid_key.get(key)?.map_or(Ok(None), |bytes| {
			let mut cross_signing_key = serde_json::from_slice::<serde_json::Value>(&bytes)
				.map_err(|e| err!(Database("CrossSigningKey in db is invalid. {e}")))?;
			clean_signatures(&mut cross_signing_key, sender_user, user_id, allowed_signatures)?;

			Ok(Some(Raw::from_json(
				serde_json::value::to_raw_value(&cross_signing_key).expect("Value to RawValue serialization"),
			)))
		})
	}

	pub(super) fn get_master_key(
		&self, sender_user: Option<&UserId>, user_id: &UserId, allowed_signatures: &dyn Fn(&UserId) -> bool,
	) -> Result<Option<Raw<CrossSigningKey>>> {
		self.userid_masterkeyid
			.get(user_id.as_bytes())?
			.map_or(Ok(None), |key| self.get_key(&key, sender_user, user_id, allowed_signatures))
	}

	pub(super) fn get_self_signing_key(
		&self, sender_user: Option<&UserId>, user_id: &UserId, allowed_signatures: &dyn Fn(&UserId) -> bool,
	) -> Result<Option<Raw<CrossSigningKey>>> {
		self.userid_selfsigningkeyid
			.get(user_id.as_bytes())?
			.map_or(Ok(None), |key| self.get_key(&key, sender_user, user_id, allowed_signatures))
	}

	pub(super) fn get_user_signing_key(&self, user_id: &UserId) -> Result<Option<Raw<CrossSigningKey>>> {
		self.userid_usersigningkeyid
			.get(user_id.as_bytes())?
			.map_or(Ok(None), |key| {
				self.keyid_key.get(&key)?.map_or(Ok(None), |bytes| {
					Ok(Some(
						serde_json::from_slice(&bytes)
							.map_err(|e| err!(Database("CrossSigningKey in db is invalid. {e}")))?,
					))
				})
			})
	}

	pub(super) fn add_to_device_event(
		&self, sender: &UserId, target_user_id: &UserId, target_device_id: &DeviceId, event_type: &str,
		content: serde_json::Value,
	) -> Result<()> {
		let mut key = target_user_id.as_bytes().to_vec();
		key.push(0xFF);
		key.extend_from_slice(target_device_id.as_bytes());
		key.push(0xFF);
		key.extend_from_slice(&self.services.globals.next_count()?.to_be_bytes());

		let mut json = serde_json::Map::new();
		json.insert("type".to_owned(), event_type.to_owned().into());
		json.insert("sender".to_owned(), sender.to_string().into());
		json.insert("content".to_owned(), content);

		let value = serde_json::to_vec(&json).expect("Map::to_vec always works");

		self.todeviceid_events.insert(&key, &value)?;

		Ok(())
	}

	pub(super) fn get_to_device_events(
		&self, user_id: &UserId, device_id: &DeviceId,
	) -> Result<Vec<Raw<AnyToDeviceEvent>>> {
		let mut events = Vec::new();

		let mut prefix = user_id.as_bytes().to_vec();
		prefix.push(0xFF);
		prefix.extend_from_slice(device_id.as_bytes());
		prefix.push(0xFF);

		for (_, value) in self.todeviceid_events.scan_prefix(prefix) {
			events.push(
				serde_json::from_slice(&value)
					.map_err(|e| err!(Database("Event in todeviceid_events is invalid. {e}")))?,
			);
		}

		Ok(events)
	}

	pub(super) fn remove_to_device_events(&self, user_id: &UserId, device_id: &DeviceId, until: u64) -> Result<()> {
		let mut prefix = user_id.as_bytes().to_vec();
		prefix.push(0xFF);
		prefix.extend_from_slice(device_id.as_bytes());
		prefix.push(0xFF);

		let mut last = prefix.clone();
		last.extend_from_slice(&until.to_be_bytes());

		for (key, _) in self
			.todeviceid_events
			.iter_from(&last, true) // this includes last
			.take_while(move |(k, _)| k.starts_with(&prefix))
			.map(|(key, _)| {
				Ok::<_, Error>((
					key.clone(),
					utils::u64_from_bytes(&key[key.len().saturating_sub(size_of::<u64>())..key.len()])
						.map_err(|e| err!(Database("ToDeviceId has invalid count bytes. {e}")))?,
				))
			})
			.filter_map(Result::ok)
			.take_while(|&(_, count)| count <= until)
		{
			self.todeviceid_events.remove(&key)?;
		}

		Ok(())
	}

	pub(super) fn update_device_metadata(&self, user_id: &UserId, device_id: &DeviceId, device: &Device) -> Result<()> {
		let mut userdeviceid = user_id.as_bytes().to_vec();
		userdeviceid.push(0xFF);
		userdeviceid.extend_from_slice(device_id.as_bytes());

		// Only existing devices should be able to call this, but we shouldn't assert
		// either...
		if self.userdeviceid_metadata.get(&userdeviceid)?.is_none() {
			warn!(
				"Called update_device_metadata for a non-existent user \"{}\" and/or device ID \"{}\" with no \
				 metadata in database",
				user_id, device_id
			);
			return Err(Error::bad_database(
				"User does not exist or device ID has no metadata in database.",
			));
		}

		self.userid_devicelistversion
			.increment(user_id.as_bytes())?;

		self.userdeviceid_metadata.insert(
			&userdeviceid,
			&serde_json::to_vec(device).expect("Device::to_string always works"),
		)?;

		Ok(())
	}

	/// Get device metadata.
	pub(super) fn get_device_metadata(&self, user_id: &UserId, device_id: &DeviceId) -> Result<Option<Device>> {
		let mut userdeviceid = user_id.as_bytes().to_vec();
		userdeviceid.push(0xFF);
		userdeviceid.extend_from_slice(device_id.as_bytes());

		self.userdeviceid_metadata
			.get(&userdeviceid)?
			.map_or(Ok(None), |bytes| {
				Ok(Some(serde_json::from_slice(&bytes).map_err(|_| {
					Error::bad_database("Metadata in userdeviceid_metadata is invalid.")
				})?))
			})
	}

	pub(super) fn get_devicelist_version(&self, user_id: &UserId) -> Result<Option<u64>> {
		self.userid_devicelistversion
			.get(user_id.as_bytes())?
			.map_or(Ok(None), |bytes| {
				utils::u64_from_bytes(&bytes)
					.map_err(|e| err!(Database("Invalid devicelistversion in db. {e}")))
					.map(Some)
			})
	}

	pub(super) fn all_devices_metadata<'a>(
		&'a self, user_id: &UserId,
	) -> Box<dyn Iterator<Item = Result<Device>> + 'a> {
		let mut key = user_id.as_bytes().to_vec();
		key.push(0xFF);

		Box::new(
			self.userdeviceid_metadata
				.scan_prefix(key)
				.map(|(_, bytes)| {
					serde_json::from_slice::<Device>(&bytes)
						.map_err(|e| err!(Database("Device in userdeviceid_metadata is invalid. {e}")))
				}),
		)
	}

	/// Creates a new sync filter. Returns the filter id.
	pub(super) fn create_filter(&self, user_id: &UserId, filter: &FilterDefinition) -> Result<String> {
		let filter_id = utils::random_string(4);

		let mut key = user_id.as_bytes().to_vec();
		key.push(0xFF);
		key.extend_from_slice(filter_id.as_bytes());

		self.userfilterid_filter
			.insert(&key, &serde_json::to_vec(&filter).expect("filter is valid json"))?;

		Ok(filter_id)
	}

	pub(super) fn get_filter(&self, user_id: &UserId, filter_id: &str) -> Result<Option<FilterDefinition>> {
		let mut key = user_id.as_bytes().to_vec();
		key.push(0xFF);
		key.extend_from_slice(filter_id.as_bytes());

		let raw = self.userfilterid_filter.get(&key)?;

		if let Some(raw) = raw {
			serde_json::from_slice(&raw).map_err(|e| err!(Database("Invalid filter event in db. {e}")))
		} else {
			Ok(None)
		}
	}

	/// Creates an OpenID token, which can be used to prove that a user has
	/// access to an account (primarily for integrations)
	pub(super) fn create_openid_token(&self, user_id: &UserId, token: &str) -> Result<u64> {
		use std::num::Saturating as Sat;

		let expires_in = self.services.server.config.openid_token_ttl;
		let expires_at = Sat(utils::millis_since_unix_epoch()) + Sat(expires_in) * Sat(1000);

		let mut value = expires_at.0.to_be_bytes().to_vec();
		value.extend_from_slice(user_id.as_bytes());

		self.openidtoken_expiresatuserid
			.insert(token.as_bytes(), value.as_slice())?;

		Ok(expires_in)
	}

	/// Find out which user an OpenID access token belongs to.
	pub(super) fn find_from_openid_token(&self, token: &str) -> Result<OwnedUserId> {
		let Some(value) = self.openidtoken_expiresatuserid.get(token.as_bytes())? else {
			return Err(Error::BadRequest(ErrorKind::Unauthorized, "OpenID token is unrecognised"));
		};

		let (expires_at_bytes, user_bytes) = value.split_at(0_u64.to_be_bytes().len());

		let expires_at = u64::from_be_bytes(
			expires_at_bytes
				.try_into()
				.map_err(|e| err!(Database("expires_at in openid_userid is invalid u64. {e}")))?,
		);

		if expires_at < utils::millis_since_unix_epoch() {
			debug_info!("OpenID token is expired, removing");
			self.openidtoken_expiresatuserid.remove(token.as_bytes())?;

			return Err(Error::BadRequest(ErrorKind::Unauthorized, "OpenID token is expired"));
		}

		UserId::parse(
			utils::string_from_bytes(user_bytes)
				.map_err(|e| err!(Database("User ID in openid_userid is invalid unicode. {e}")))?,
		)
		.map_err(|e| err!(Database("User ID in openid_userid is invalid. {e}")))
	}
}

/// Will only return with Some(username) if the password was not empty and the
/// username could be successfully parsed.
/// If `utils::string_from_bytes`(...) returns an error that username will be
/// skipped and the error will be logged.
pub(super) fn get_username_with_valid_password(username: &[u8], password: &[u8]) -> Option<String> {
	// A valid password is not empty
	if password.is_empty() {
		None
	} else {
		match utils::string_from_bytes(username) {
			Ok(u) => Some(u),
			Err(e) => {
				warn!("Failed to parse username while calling get_local_users(): {}", e.to_string());
				None
			},
		}
	}
}
