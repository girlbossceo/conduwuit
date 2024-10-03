use std::{collections::BTreeMap, mem, mem::size_of, sync::Arc};

use conduit::{
	debug_warn, err, utils,
	utils::{stream::TryIgnore, string::Unquoted, ReadyExt},
	warn, Err, Error, Result, Server,
};
use database::{Deserialized, Ignore, Interfix, Map};
use futures::{pin_mut, FutureExt, Stream, StreamExt, TryFutureExt};
use ruma::{
	api::client::{device::Device, error::ErrorKind, filter::FilterDefinition},
	encryption::{CrossSigningKey, DeviceKeys, OneTimeKey},
	events::{ignored_user_list::IgnoredUserListEvent, AnyToDeviceEvent, GlobalAccountDataEventType, StateEventType},
	serde::Raw,
	DeviceId, DeviceKeyAlgorithm, DeviceKeyId, MilliSecondsSinceUnixEpoch, OwnedDeviceId, OwnedDeviceKeyId,
	OwnedMxcUri, OwnedUserId, UInt, UserId,
};

use crate::{account_data, admin, globals, rooms, Dep};

pub struct Service {
	services: Services,
	db: Data,
}

struct Services {
	server: Arc<Server>,
	account_data: Dep<account_data::Service>,
	admin: Dep<admin::Service>,
	globals: Dep<globals::Service>,
	state_accessor: Dep<rooms::state_accessor::Service>,
	state_cache: Dep<rooms::state_cache::Service>,
}

struct Data {
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
	useridprofilekey_value: Arc<Map>,
}

impl crate::Service for Service {
	fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
		Ok(Arc::new(Self {
			services: Services {
				server: args.server.clone(),
				account_data: args.depend::<account_data::Service>("account_data"),
				admin: args.depend::<admin::Service>("admin"),
				globals: args.depend::<globals::Service>("globals"),
				state_accessor: args.depend::<rooms::state_accessor::Service>("rooms::state_accessor"),
				state_cache: args.depend::<rooms::state_cache::Service>("rooms::state_cache"),
			},
			db: Data {
				keychangeid_userid: args.db["keychangeid_userid"].clone(),
				keyid_key: args.db["keyid_key"].clone(),
				onetimekeyid_onetimekeys: args.db["onetimekeyid_onetimekeys"].clone(),
				openidtoken_expiresatuserid: args.db["openidtoken_expiresatuserid"].clone(),
				todeviceid_events: args.db["todeviceid_events"].clone(),
				token_userdeviceid: args.db["token_userdeviceid"].clone(),
				userdeviceid_metadata: args.db["userdeviceid_metadata"].clone(),
				userdeviceid_token: args.db["userdeviceid_token"].clone(),
				userfilterid_filter: args.db["userfilterid_filter"].clone(),
				userid_avatarurl: args.db["userid_avatarurl"].clone(),
				userid_blurhash: args.db["userid_blurhash"].clone(),
				userid_devicelistversion: args.db["userid_devicelistversion"].clone(),
				userid_displayname: args.db["userid_displayname"].clone(),
				userid_lastonetimekeyupdate: args.db["userid_lastonetimekeyupdate"].clone(),
				userid_masterkeyid: args.db["userid_masterkeyid"].clone(),
				userid_password: args.db["userid_password"].clone(),
				userid_selfsigningkeyid: args.db["userid_selfsigningkeyid"].clone(),
				userid_usersigningkeyid: args.db["userid_usersigningkeyid"].clone(),
				useridprofilekey_value: args.db["useridprofilekey_value"].clone(),
			},
		}))
	}

	fn name(&self) -> &str { crate::service::make_name(std::module_path!()) }
}

impl Service {
	/// Returns true/false based on whether the recipient/receiving user has
	/// blocked the sender
	pub async fn user_is_ignored(&self, sender_user: &UserId, recipient_user: &UserId) -> bool {
		self.services
			.account_data
			.get_global(recipient_user, GlobalAccountDataEventType::IgnoredUserList)
			.await
			.map_or(false, |ignored: IgnoredUserListEvent| {
				ignored
					.content
					.ignored_users
					.keys()
					.any(|blocked_user| blocked_user == sender_user)
			})
	}

	/// Check if a user is an admin
	#[inline]
	pub async fn is_admin(&self, user_id: &UserId) -> bool { self.services.admin.user_is_admin(user_id).await }

	/// Create a new user account on this homeserver.
	#[inline]
	pub fn create(&self, user_id: &UserId, password: Option<&str>) -> Result<()> {
		self.set_password(user_id, password)
	}

	/// Deactivate account
	pub async fn deactivate_account(&self, user_id: &UserId) -> Result<()> {
		// Remove all associated devices
		self.all_device_ids(user_id)
			.for_each(|device_id| self.remove_device(user_id, device_id))
			.await;

		// Set the password to "" to indicate a deactivated account. Hashes will never
		// result in an empty string, so the user will not be able to log in again.
		// Systems like changing the password without logging in should check if the
		// account is deactivated.
		self.set_password(user_id, None)?;

		// TODO: Unhook 3PID
		Ok(())
	}

	/// Check if a user has an account on this homeserver.
	#[inline]
	pub async fn exists(&self, user_id: &UserId) -> bool { self.db.userid_password.get(user_id).await.is_ok() }

	/// Check if account is deactivated
	pub async fn is_deactivated(&self, user_id: &UserId) -> Result<bool> {
		self.db
			.userid_password
			.get(user_id)
			.map_ok(|val| val.is_empty())
			.map_err(|_| err!(Request(NotFound("User does not exist."))))
			.await
	}

	/// Check if account is active, infallible
	pub async fn is_active(&self, user_id: &UserId) -> bool { !self.is_deactivated(user_id).await.unwrap_or(true) }

	/// Check if account is active, infallible
	pub async fn is_active_local(&self, user_id: &UserId) -> bool {
		self.services.globals.user_is_local(user_id) && self.is_active(user_id).await
	}

	/// Returns the number of users registered on this server.
	#[inline]
	pub async fn count(&self) -> usize { self.db.userid_password.count().await }

	/// Find out which user an access token belongs to.
	pub async fn find_from_token(&self, token: &str) -> Result<(OwnedUserId, OwnedDeviceId)> {
		self.db.token_userdeviceid.get(token).await.deserialized()
	}

	/// Returns an iterator over all users on this homeserver (offered for
	/// compatibility)
	#[allow(clippy::iter_without_into_iter, clippy::iter_not_returning_iterator)]
	pub fn iter(&self) -> impl Stream<Item = OwnedUserId> + Send + '_ { self.stream().map(ToOwned::to_owned) }

	/// Returns an iterator over all users on this homeserver.
	pub fn stream(&self) -> impl Stream<Item = &UserId> + Send { self.db.userid_password.keys().ignore_err() }

	/// Returns a list of local users as list of usernames.
	///
	/// A user account is considered `local` if the length of it's password is
	/// greater then zero.
	pub fn list_local_users(&self) -> impl Stream<Item = &UserId> + Send + '_ {
		self.db
			.userid_password
			.stream()
			.ignore_err()
			.ready_filter_map(|(u, p): (&UserId, &[u8])| (!p.is_empty()).then_some(u))
	}

	/// Returns the password hash for the given user.
	pub async fn password_hash(&self, user_id: &UserId) -> Result<String> {
		self.db.userid_password.get(user_id).await.deserialized()
	}

	/// Hash and set the user's password to the Argon2 hash
	pub fn set_password(&self, user_id: &UserId, password: Option<&str>) -> Result<()> {
		if let Some(password) = password {
			if let Ok(hash) = utils::hash::password(password) {
				self.db
					.userid_password
					.insert(user_id.as_bytes(), hash.as_bytes());
				Ok(())
			} else {
				Err(Error::BadRequest(
					ErrorKind::InvalidParam,
					"Password does not meet the requirements.",
				))
			}
		} else {
			self.db.userid_password.insert(user_id.as_bytes(), b"");
			Ok(())
		}
	}

	/// Returns the displayname of a user on this homeserver.
	pub async fn displayname(&self, user_id: &UserId) -> Result<String> {
		self.db.userid_displayname.get(user_id).await.deserialized()
	}

	/// Sets a new displayname or removes it if displayname is None. You still
	/// need to nofify all rooms of this change.
	pub fn set_displayname(&self, user_id: &UserId, displayname: Option<String>) {
		if let Some(displayname) = displayname {
			self.db
				.userid_displayname
				.insert(user_id.as_bytes(), displayname.as_bytes());
		} else {
			self.db.userid_displayname.remove(user_id.as_bytes());
		}
	}

	/// Get the `avatar_url` of a user.
	pub async fn avatar_url(&self, user_id: &UserId) -> Result<OwnedMxcUri> {
		self.db.userid_avatarurl.get(user_id).await.deserialized()
	}

	/// Sets a new avatar_url or removes it if avatar_url is None.
	pub fn set_avatar_url(&self, user_id: &UserId, avatar_url: Option<OwnedMxcUri>) {
		if let Some(avatar_url) = avatar_url {
			self.db
				.userid_avatarurl
				.insert(user_id.as_bytes(), avatar_url.to_string().as_bytes());
		} else {
			self.db.userid_avatarurl.remove(user_id.as_bytes());
		}
	}

	/// Get the blurhash of a user.
	pub async fn blurhash(&self, user_id: &UserId) -> Result<String> {
		self.db.userid_blurhash.get(user_id).await.deserialized()
	}

	/// Sets a new avatar_url or removes it if avatar_url is None.
	pub fn set_blurhash(&self, user_id: &UserId, blurhash: Option<String>) {
		if let Some(blurhash) = blurhash {
			self.db
				.userid_blurhash
				.insert(user_id.as_bytes(), blurhash.as_bytes());
		} else {
			self.db.userid_blurhash.remove(user_id.as_bytes());
		}
	}

	/// Adds a new device to a user.
	pub async fn create_device(
		&self, user_id: &UserId, device_id: &DeviceId, token: &str, initial_device_display_name: Option<String>,
		client_ip: Option<String>,
	) -> Result<()> {
		// This method should never be called for nonexistent users. We shouldn't assert
		// though...
		if !self.exists(user_id).await {
			warn!("Called create_device for non-existent user {} in database", user_id);
			return Err(Error::BadRequest(ErrorKind::InvalidParam, "User does not exist."));
		}

		let mut userdeviceid = user_id.as_bytes().to_vec();
		userdeviceid.push(0xFF);
		userdeviceid.extend_from_slice(device_id.as_bytes());

		increment(&self.db.userid_devicelistversion, user_id.as_bytes());

		self.db.userdeviceid_metadata.insert(
			&userdeviceid,
			&serde_json::to_vec(&Device {
				device_id: device_id.into(),
				display_name: initial_device_display_name,
				last_seen_ip: client_ip,
				last_seen_ts: Some(MilliSecondsSinceUnixEpoch::now()),
			})
			.expect("Device::to_string never fails."),
		);

		self.set_token(user_id, device_id, token).await?;

		Ok(())
	}

	/// Removes a device from a user.
	pub async fn remove_device(&self, user_id: &UserId, device_id: &DeviceId) {
		let mut userdeviceid = user_id.as_bytes().to_vec();
		userdeviceid.push(0xFF);
		userdeviceid.extend_from_slice(device_id.as_bytes());

		// Remove tokens
		if let Ok(old_token) = self.db.userdeviceid_token.get(&userdeviceid).await {
			self.db.userdeviceid_token.remove(&userdeviceid);
			self.db.token_userdeviceid.remove(&old_token);
		}

		// Remove todevice events
		let prefix = (user_id, device_id, Interfix);
		self.db
			.todeviceid_events
			.keys_raw_prefix(&prefix)
			.ignore_err()
			.ready_for_each(|key| self.db.todeviceid_events.remove(key))
			.await;

		// TODO: Remove onetimekeys

		increment(&self.db.userid_devicelistversion, user_id.as_bytes());

		self.db.userdeviceid_metadata.remove(&userdeviceid);
	}

	/// Returns an iterator over all device ids of this user.
	pub fn all_device_ids<'a>(&'a self, user_id: &'a UserId) -> impl Stream<Item = &DeviceId> + Send + 'a {
		let prefix = (user_id, Interfix);
		self.db
			.userdeviceid_metadata
			.keys_prefix(&prefix)
			.ignore_err()
			.map(|(_, device_id): (Ignore, &DeviceId)| device_id)
	}

	/// Replaces the access token of one device.
	pub async fn set_token(&self, user_id: &UserId, device_id: &DeviceId, token: &str) -> Result<()> {
		let key = (user_id, device_id);
		// should not be None, but we shouldn't assert either lol...
		if self.db.userdeviceid_metadata.qry(&key).await.is_err() {
			return Err!(Database(error!(
				?user_id,
				?device_id,
				"User does not exist or device has no metadata."
			)));
		}

		// Remove old token
		if let Ok(old_token) = self.db.userdeviceid_token.qry(&key).await {
			self.db.token_userdeviceid.remove(&old_token);
			// It will be removed from userdeviceid_token by the insert later
		}

		// Assign token to user device combination
		let mut userdeviceid = user_id.as_bytes().to_vec();
		userdeviceid.push(0xFF);
		userdeviceid.extend_from_slice(device_id.as_bytes());
		self.db
			.userdeviceid_token
			.insert(&userdeviceid, token.as_bytes());
		self.db
			.token_userdeviceid
			.insert(token.as_bytes(), &userdeviceid);

		Ok(())
	}

	pub async fn add_one_time_key(
		&self, user_id: &UserId, device_id: &DeviceId, one_time_key_key: &DeviceKeyId,
		one_time_key_value: &Raw<OneTimeKey>,
	) -> Result<()> {
		// All devices have metadata
		// Only existing devices should be able to call this, but we shouldn't assert
		// either...
		let key = (user_id, device_id);
		if self.db.userdeviceid_metadata.qry(&key).await.is_err() {
			return Err!(Database(error!(
				?user_id,
				?device_id,
				"User does not exist or device has no metadata."
			)));
		}

		let mut key = user_id.as_bytes().to_vec();
		key.push(0xFF);
		key.extend_from_slice(device_id.as_bytes());
		key.push(0xFF);
		// TODO: Use DeviceKeyId::to_string when it's available (and update everything,
		// because there are no wrapping quotation marks anymore)
		key.extend_from_slice(
			serde_json::to_string(one_time_key_key)
				.expect("DeviceKeyId::to_string always works")
				.as_bytes(),
		);

		self.db.onetimekeyid_onetimekeys.insert(
			&key,
			&serde_json::to_vec(&one_time_key_value).expect("OneTimeKey::to_vec always works"),
		);

		self.db
			.userid_lastonetimekeyupdate
			.insert(user_id.as_bytes(), &self.services.globals.next_count()?.to_be_bytes());

		Ok(())
	}

	pub async fn last_one_time_keys_update(&self, user_id: &UserId) -> u64 {
		self.db
			.userid_lastonetimekeyupdate
			.get(user_id)
			.await
			.deserialized()
			.unwrap_or(0)
	}

	pub async fn take_one_time_key(
		&self, user_id: &UserId, device_id: &DeviceId, key_algorithm: &DeviceKeyAlgorithm,
	) -> Result<(OwnedDeviceKeyId, Raw<OneTimeKey>)> {
		self.db
			.userid_lastonetimekeyupdate
			.insert(user_id.as_bytes(), &self.services.globals.next_count()?.to_be_bytes());

		let mut prefix = user_id.as_bytes().to_vec();
		prefix.push(0xFF);
		prefix.extend_from_slice(device_id.as_bytes());
		prefix.push(0xFF);
		prefix.push(b'"'); // Annoying quotation mark
		prefix.extend_from_slice(key_algorithm.as_ref().as_bytes());
		prefix.push(b':');

		let one_time_key = self
			.db
			.onetimekeyid_onetimekeys
			.raw_stream_prefix(&prefix)
			.ignore_err()
			.map(|(key, val)| {
				self.db.onetimekeyid_onetimekeys.remove(key);

				let key = key
					.rsplit(|&b| b == 0xFF)
					.next()
					.ok_or_else(|| err!(Database("OneTimeKeyId in db is invalid.")))
					.unwrap();

				let key = serde_json::from_slice(key)
					.map_err(|e| err!(Database("OneTimeKeyId in db is invalid. {e}")))
					.unwrap();

				let val = serde_json::from_slice(val)
					.map_err(|e| err!(Database("OneTimeKeys in db are invalid. {e}")))
					.unwrap();

				(key, val)
			})
			.next()
			.await;

		one_time_key.ok_or_else(|| err!(Request(NotFound("No one-time-key found"))))
	}

	pub async fn count_one_time_keys(
		&self, user_id: &UserId, device_id: &DeviceId,
	) -> BTreeMap<DeviceKeyAlgorithm, UInt> {
		type KeyVal<'a> = ((Ignore, Ignore, &'a Unquoted), Ignore);

		let mut algorithm_counts = BTreeMap::<DeviceKeyAlgorithm, UInt>::new();
		let query = (user_id, device_id);
		self.db
			.onetimekeyid_onetimekeys
			.stream_prefix(&query)
			.ignore_err()
			.ready_for_each(|((Ignore, Ignore, device_key_id), Ignore): KeyVal<'_>| {
				let device_key_id: &DeviceKeyId = device_key_id
					.as_str()
					.try_into()
					.expect("Invalid DeviceKeyID in database");

				let count: &mut UInt = algorithm_counts
					.entry(device_key_id.algorithm())
					.or_default();

				*count = count.saturating_add(1_u32.into());
			})
			.await;

		algorithm_counts
	}

	pub async fn add_device_keys(&self, user_id: &UserId, device_id: &DeviceId, device_keys: &Raw<DeviceKeys>) {
		let mut userdeviceid = user_id.as_bytes().to_vec();
		userdeviceid.push(0xFF);
		userdeviceid.extend_from_slice(device_id.as_bytes());

		self.db.keyid_key.insert(
			&userdeviceid,
			&serde_json::to_vec(&device_keys).expect("DeviceKeys::to_vec always works"),
		);

		self.mark_device_key_update(user_id).await;
	}

	pub async fn add_cross_signing_keys(
		&self, user_id: &UserId, master_key: &Raw<CrossSigningKey>, self_signing_key: &Option<Raw<CrossSigningKey>>,
		user_signing_key: &Option<Raw<CrossSigningKey>>, notify: bool,
	) -> Result<()> {
		// TODO: Check signatures
		let mut prefix = user_id.as_bytes().to_vec();
		prefix.push(0xFF);

		let (master_key_key, _) = parse_master_key(user_id, master_key)?;

		self.db
			.keyid_key
			.insert(&master_key_key, master_key.json().get().as_bytes());

		self.db
			.userid_masterkeyid
			.insert(user_id.as_bytes(), &master_key_key);

		// Self-signing key
		if let Some(self_signing_key) = self_signing_key {
			let mut self_signing_key_ids = self_signing_key
				.deserialize()
				.map_err(|e| err!(Request(InvalidParam("Invalid self signing key: {e:?}"))))?
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

			self.db
				.keyid_key
				.insert(&self_signing_key_key, self_signing_key.json().get().as_bytes());

			self.db
				.userid_selfsigningkeyid
				.insert(user_id.as_bytes(), &self_signing_key_key);
		}

		// User-signing key
		if let Some(user_signing_key) = user_signing_key {
			let mut user_signing_key_ids = user_signing_key
				.deserialize()
				.map_err(|_| err!(Request(InvalidParam("Invalid user signing key"))))?
				.keys
				.into_values();

			let user_signing_key_id = user_signing_key_ids
				.next()
				.ok_or(err!(Request(InvalidParam("User signing key contained no key."))))?;

			if user_signing_key_ids.next().is_some() {
				return Err!(Request(InvalidParam("User signing key contained more than one key.")));
			}

			let mut user_signing_key_key = prefix;
			user_signing_key_key.extend_from_slice(user_signing_key_id.as_bytes());

			self.db
				.keyid_key
				.insert(&user_signing_key_key, user_signing_key.json().get().as_bytes());

			self.db
				.userid_usersigningkeyid
				.insert(user_id.as_bytes(), &user_signing_key_key);
		}

		if notify {
			self.mark_device_key_update(user_id).await;
		}

		Ok(())
	}

	pub async fn sign_key(
		&self, target_id: &UserId, key_id: &str, signature: (String, String), sender_id: &UserId,
	) -> Result<()> {
		let key = (target_id, key_id);

		let mut cross_signing_key: serde_json::Value = self
			.db
			.keyid_key
			.qry(&key)
			.await
			.map_err(|_| err!(Request(InvalidParam("Tried to sign nonexistent key."))))?
			.deserialized()
			.map_err(|e| err!(Database("key in keyid_key is invalid. {e:?}")))?;

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

		let mut key = target_id.as_bytes().to_vec();
		key.push(0xFF);
		key.extend_from_slice(key_id.as_bytes());
		self.db.keyid_key.insert(
			&key,
			&serde_json::to_vec(&cross_signing_key).expect("CrossSigningKey::to_vec always works"),
		);

		self.mark_device_key_update(target_id).await;

		Ok(())
	}

	pub fn keys_changed<'a>(
		&'a self, user_or_room_id: &'a str, from: u64, to: Option<u64>,
	) -> impl Stream<Item = &UserId> + Send + 'a {
		type KeyVal<'a> = ((&'a str, u64), &'a UserId);

		let to = to.unwrap_or(u64::MAX);
		let start = (user_or_room_id, from.saturating_add(1));
		self.db
			.keychangeid_userid
			.stream_from(&start)
			.ignore_err()
			.ready_take_while(move |((prefix, count), _): &KeyVal<'_>| *prefix == user_or_room_id && *count <= to)
			.map(|((..), user_id): KeyVal<'_>| user_id)
	}

	pub async fn mark_device_key_update(&self, user_id: &UserId) {
		let count = self.services.globals.next_count().unwrap().to_be_bytes();

		let rooms_joined = self.services.state_cache.rooms_joined(user_id);

		pin_mut!(rooms_joined);
		while let Some(room_id) = rooms_joined.next().await {
			// Don't send key updates to unencrypted rooms
			if self
				.services
				.state_accessor
				.room_state_get(room_id, &StateEventType::RoomEncryption, "")
				.await
				.is_err()
			{
				continue;
			}

			let mut key = room_id.as_bytes().to_vec();
			key.push(0xFF);
			key.extend_from_slice(&count);

			self.db.keychangeid_userid.insert(&key, user_id.as_bytes());
		}

		let mut key = user_id.as_bytes().to_vec();
		key.push(0xFF);
		key.extend_from_slice(&count);
		self.db.keychangeid_userid.insert(&key, user_id.as_bytes());
	}

	pub async fn get_device_keys<'a>(&'a self, user_id: &'a UserId, device_id: &DeviceId) -> Result<Raw<DeviceKeys>> {
		let key_id = (user_id, device_id);
		self.db.keyid_key.qry(&key_id).await.deserialized()
	}

	pub async fn get_key<F>(
		&self, key_id: &[u8], sender_user: Option<&UserId>, user_id: &UserId, allowed_signatures: &F,
	) -> Result<Raw<CrossSigningKey>>
	where
		F: Fn(&UserId) -> bool + Send + Sync,
	{
		let key = self
			.db
			.keyid_key
			.get(key_id)
			.await
			.deserialized::<serde_json::Value>()?;

		let cleaned = clean_signatures(key, sender_user, user_id, allowed_signatures)?;
		let raw_value = serde_json::value::to_raw_value(&cleaned)?;
		Ok(Raw::from_json(raw_value))
	}

	pub async fn get_master_key<F>(
		&self, sender_user: Option<&UserId>, user_id: &UserId, allowed_signatures: &F,
	) -> Result<Raw<CrossSigningKey>>
	where
		F: Fn(&UserId) -> bool + Send + Sync,
	{
		let key_id = self.db.userid_masterkeyid.get(user_id).await?;

		self.get_key(&key_id, sender_user, user_id, allowed_signatures)
			.await
	}

	pub async fn get_self_signing_key<F>(
		&self, sender_user: Option<&UserId>, user_id: &UserId, allowed_signatures: &F,
	) -> Result<Raw<CrossSigningKey>>
	where
		F: Fn(&UserId) -> bool + Send + Sync,
	{
		let key_id = self.db.userid_selfsigningkeyid.get(user_id).await?;

		self.get_key(&key_id, sender_user, user_id, allowed_signatures)
			.await
	}

	pub async fn get_user_signing_key(&self, user_id: &UserId) -> Result<Raw<CrossSigningKey>> {
		let key_id = self.db.userid_usersigningkeyid.get(user_id).await?;

		self.db.keyid_key.get(&*key_id).await.deserialized()
	}

	pub async fn add_to_device_event(
		&self, sender: &UserId, target_user_id: &UserId, target_device_id: &DeviceId, event_type: &str,
		content: serde_json::Value,
	) {
		let mut key = target_user_id.as_bytes().to_vec();
		key.push(0xFF);
		key.extend_from_slice(target_device_id.as_bytes());
		key.push(0xFF);
		key.extend_from_slice(&self.services.globals.next_count().unwrap().to_be_bytes());

		let mut json = serde_json::Map::new();
		json.insert("type".to_owned(), event_type.to_owned().into());
		json.insert("sender".to_owned(), sender.to_string().into());
		json.insert("content".to_owned(), content);

		let value = serde_json::to_vec(&json).expect("Map::to_vec always works");

		self.db.todeviceid_events.insert(&key, &value);
	}

	pub fn get_to_device_events<'a>(
		&'a self, user_id: &'a UserId, device_id: &'a DeviceId,
	) -> impl Stream<Item = Raw<AnyToDeviceEvent>> + Send + 'a {
		let prefix = (user_id, device_id, Interfix);
		self.db
			.todeviceid_events
			.stream_prefix(&prefix)
			.ignore_err()
			.map(|(_, val): (Ignore, Raw<AnyToDeviceEvent>)| val)
	}

	pub async fn remove_to_device_events(&self, user_id: &UserId, device_id: &DeviceId, until: u64) {
		let mut prefix = user_id.as_bytes().to_vec();
		prefix.push(0xFF);
		prefix.extend_from_slice(device_id.as_bytes());
		prefix.push(0xFF);

		let mut last = prefix.clone();
		last.extend_from_slice(&until.to_be_bytes());

		self.db
			.todeviceid_events
			.rev_raw_keys_from(&last) // this includes last
			.ignore_err()
			.ready_take_while(move |key| key.starts_with(&prefix))
			.map(|key| {
				let len = key.len();
				let start = len.saturating_sub(size_of::<u64>());
				let count = utils::u64_from_u8(&key[start..len]);
				(key, count)
			})
			.ready_take_while(move |(_, count)| *count <= until)
			.ready_for_each(|(key, _)| self.db.todeviceid_events.remove(&key))
			.boxed()
			.await;
	}

	pub async fn update_device_metadata(&self, user_id: &UserId, device_id: &DeviceId, device: &Device) -> Result<()> {
		increment(&self.db.userid_devicelistversion, user_id.as_bytes());

		let mut userdeviceid = user_id.as_bytes().to_vec();
		userdeviceid.push(0xFF);
		userdeviceid.extend_from_slice(device_id.as_bytes());
		self.db.userdeviceid_metadata.insert(
			&userdeviceid,
			&serde_json::to_vec(device).expect("Device::to_string always works"),
		);

		Ok(())
	}

	/// Get device metadata.
	pub async fn get_device_metadata(&self, user_id: &UserId, device_id: &DeviceId) -> Result<Device> {
		self.db
			.userdeviceid_metadata
			.qry(&(user_id, device_id))
			.await
			.deserialized()
	}

	pub async fn get_devicelist_version(&self, user_id: &UserId) -> Result<u64> {
		self.db
			.userid_devicelistversion
			.get(user_id)
			.await
			.deserialized()
	}

	pub fn all_devices_metadata<'a>(&'a self, user_id: &'a UserId) -> impl Stream<Item = Device> + Send + 'a {
		let key = (user_id, Interfix);
		self.db
			.userdeviceid_metadata
			.stream_prefix(&key)
			.ignore_err()
			.map(|(_, val): (Ignore, Device)| val)
	}

	/// Creates a new sync filter. Returns the filter id.
	pub fn create_filter(&self, user_id: &UserId, filter: &FilterDefinition) -> String {
		let filter_id = utils::random_string(4);

		let mut key = user_id.as_bytes().to_vec();
		key.push(0xFF);
		key.extend_from_slice(filter_id.as_bytes());

		self.db
			.userfilterid_filter
			.insert(&key, &serde_json::to_vec(&filter).expect("filter is valid json"));

		filter_id
	}

	pub async fn get_filter(&self, user_id: &UserId, filter_id: &str) -> Result<FilterDefinition> {
		self.db
			.userfilterid_filter
			.qry(&(user_id, filter_id))
			.await
			.deserialized()
	}

	/// Creates an OpenID token, which can be used to prove that a user has
	/// access to an account (primarily for integrations)
	pub fn create_openid_token(&self, user_id: &UserId, token: &str) -> Result<u64> {
		use std::num::Saturating as Sat;

		let expires_in = self.services.server.config.openid_token_ttl;
		let expires_at = Sat(utils::millis_since_unix_epoch()) + Sat(expires_in) * Sat(1000);

		let mut value = expires_at.0.to_be_bytes().to_vec();
		value.extend_from_slice(user_id.as_bytes());

		self.db
			.openidtoken_expiresatuserid
			.insert(token.as_bytes(), value.as_slice());

		Ok(expires_in)
	}

	/// Find out which user an OpenID access token belongs to.
	pub async fn find_from_openid_token(&self, token: &str) -> Result<OwnedUserId> {
		let Ok(value) = self.db.openidtoken_expiresatuserid.get(token).await else {
			return Err!(Request(Unauthorized("OpenID token is unrecognised")));
		};

		let (expires_at_bytes, user_bytes) = value.split_at(0_u64.to_be_bytes().len());
		let expires_at = u64::from_be_bytes(
			expires_at_bytes
				.try_into()
				.map_err(|e| err!(Database("expires_at in openid_userid is invalid u64. {e}")))?,
		);

		if expires_at < utils::millis_since_unix_epoch() {
			debug_warn!("OpenID token is expired, removing");
			self.db.openidtoken_expiresatuserid.remove(token.as_bytes());

			return Err!(Request(Unauthorized("OpenID token is expired")));
		}

		let user_string = utils::string_from_bytes(user_bytes)
			.map_err(|e| err!(Database("User ID in openid_userid is invalid unicode. {e}")))?;

		UserId::parse(user_string).map_err(|e| err!(Database("User ID in openid_userid is invalid. {e}")))
	}

	/// Gets a specific user profile key
	pub async fn profile_key(&self, user_id: &UserId, profile_key: &str) -> Result<serde_json::Value> {
		let key = (user_id, profile_key);
		self.db
			.useridprofilekey_value
			.qry(&key)
			.await
			.deserialized()
	}

	/// Gets all the user's profile keys and values in an iterator
	pub fn all_profile_keys<'a>(
		&'a self, user_id: &'a UserId,
	) -> impl Stream<Item = (String, serde_json::Value)> + 'a + Send {
		type KeyVal = ((Ignore, String), serde_json::Value);

		let prefix = (user_id, Interfix);
		self.db
			.useridprofilekey_value
			.stream_prefix(&prefix)
			.ignore_err()
			.map(|((_, key), val): KeyVal| (key, val))
	}

	/// Sets a new profile key value, removes the key if value is None
	pub fn set_profile_key(&self, user_id: &UserId, profile_key: &str, profile_key_value: Option<serde_json::Value>) {
		let mut key = user_id.as_bytes().to_vec();
		key.push(0xFF);
		key.extend_from_slice(profile_key.as_bytes());

		// TODO: insert to the stable MSC4175 key when it's stable
		if let Some(value) = profile_key_value {
			let value = serde_json::to_vec(&value).unwrap();

			self.db.useridprofilekey_value.insert(&key, &value);
		} else {
			self.db.useridprofilekey_value.remove(&key);
		}
	}

	/// Get the timezone of a user.
	pub async fn timezone(&self, user_id: &UserId) -> Result<String> {
		// TODO: transparently migrate unstable key usage to the stable key once MSC4133
		// and MSC4175 are stable, likely a remove/insert in this block.

		// first check the unstable prefix then check the stable prefix
		let unstable_key = (user_id, "us.cloke.msc4175.tz");
		let stable_key = (user_id, "m.tz");
		self.db
			.useridprofilekey_value
			.qry(&unstable_key)
			.or_else(|_| self.db.useridprofilekey_value.qry(&stable_key))
			.await
			.deserialized()
	}

	/// Sets a new timezone or removes it if timezone is None.
	pub fn set_timezone(&self, user_id: &UserId, timezone: Option<String>) {
		let mut key = user_id.as_bytes().to_vec();
		key.push(0xFF);
		key.extend_from_slice(b"us.cloke.msc4175.tz");

		// TODO: insert to the stable MSC4175 key when it's stable
		if let Some(timezone) = timezone {
			self.db
				.useridprofilekey_value
				.insert(&key, timezone.as_bytes());
		} else {
			self.db.useridprofilekey_value.remove(&key);
		}
	}
}

pub fn parse_master_key(user_id: &UserId, master_key: &Raw<CrossSigningKey>) -> Result<(Vec<u8>, CrossSigningKey)> {
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

/// Ensure that a user only sees signatures from themselves and the target user
fn clean_signatures<F>(
	mut cross_signing_key: serde_json::Value, sender_user: Option<&UserId>, user_id: &UserId, allowed_signatures: &F,
) -> Result<serde_json::Value>
where
	F: Fn(&UserId) -> bool + Send + Sync,
{
	if let Some(signatures) = cross_signing_key
		.get_mut("signatures")
		.and_then(|v| v.as_object_mut())
	{
		// Don't allocate for the full size of the current signatures, but require
		// at most one resize if nothing is dropped
		let new_capacity = signatures.len() / 2;
		for (user, signature) in mem::replace(signatures, serde_json::Map::with_capacity(new_capacity)) {
			let sid =
				<&UserId>::try_from(user.as_str()).map_err(|_| Error::bad_database("Invalid user ID in database."))?;
			if sender_user == Some(user_id) || sid == user_id || allowed_signatures(sid) {
				signatures.insert(user, signature);
			}
		}
	}

	Ok(cross_signing_key)
}

//TODO: this is an ABA
fn increment(db: &Arc<Map>, key: &[u8]) {
	let old = db.get_blocking(key);
	let new = utils::increment(old.ok().as_deref());
	db.insert(key, &new);
}
