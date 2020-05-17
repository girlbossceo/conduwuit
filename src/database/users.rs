use crate::{utils, Error, Result};
use js_int::UInt;
use ruma_client_api::r0::keys::{AlgorithmAndDeviceId, DeviceKeys, KeyAlgorithm, OneTimeKey};
use ruma_events::{to_device::AnyToDeviceEvent, EventJson, EventType};
use ruma_identifiers::{DeviceId, UserId};
use std::{collections::BTreeMap, convert::TryFrom};

pub struct Users {
    pub(super) userid_password: sled::Tree,
    pub(super) userid_displayname: sled::Tree,
    pub(super) userid_avatarurl: sled::Tree,
    pub(super) userdeviceids: sled::Tree,
    pub(super) userdeviceid_token: sled::Tree,
    pub(super) token_userdeviceid: sled::Tree,

    pub(super) onetimekeyid_onetimekeys: sled::Tree, // OneTimeKeyId = UserId + AlgorithmAndDeviceId
    pub(super) userdeviceid_devicekeys: sled::Tree,

    pub(super) todeviceid_events: sled::Tree, // ToDeviceId = UserId + DeviceId + Count
}

impl Users {
    /// Check if a user has an account on this homeserver.
    pub fn exists(&self, user_id: &UserId) -> Result<bool> {
        Ok(self.userid_password.contains_key(user_id.to_string())?)
    }

    /// Create a new user account on this homeserver.
    pub fn create(&self, user_id: &UserId, hash: &str) -> Result<()> {
        self.userid_password.insert(user_id.to_string(), hash)?;
        Ok(())
    }

    /// Find out which user an access token belongs to.
    pub fn find_from_token(&self, token: &str) -> Result<Option<(UserId, String)>> {
        self.token_userdeviceid
            .get(token)?
            .map_or(Ok(None), |bytes| {
                let mut parts = bytes.split(|&b| b == 0xff);
                let user_bytes = parts
                    .next()
                    .ok_or(Error::BadDatabase("token_userdeviceid value invalid"))?;
                let device_bytes = parts
                    .next()
                    .ok_or(Error::BadDatabase("token_userdeviceid value invalid"))?;

                Ok(Some((
                    UserId::try_from(utils::string_from_bytes(&user_bytes)?)?,
                    utils::string_from_bytes(&device_bytes)?,
                )))
            })
    }

    /// Returns an iterator over all users on this homeserver.
    pub fn iter(&self) -> impl Iterator<Item = Result<UserId>> {
        self.userid_password.iter().keys().map(|r| {
            utils::string_from_bytes(&r?).and_then(|string| Ok(UserId::try_from(&*string)?))
        })
    }

    /// Returns the password hash for the given user.
    pub fn password_hash(&self, user_id: &UserId) -> Result<Option<String>> {
        self.userid_password
            .get(user_id.to_string())?
            .map_or(Ok(None), |bytes| utils::string_from_bytes(&bytes).map(Some))
    }

    /// Returns the displayname of a user on this homeserver.
    pub fn displayname(&self, user_id: &UserId) -> Result<Option<String>> {
        self.userid_displayname
            .get(user_id.to_string())?
            .map_or(Ok(None), |bytes| utils::string_from_bytes(&bytes).map(Some))
    }

    /// Sets a new displayname or removes it if displayname is None. You still need to nofify all rooms of this change.
    pub fn set_displayname(&self, user_id: &UserId, displayname: Option<String>) -> Result<()> {
        if let Some(displayname) = displayname {
            self.userid_displayname
                .insert(user_id.to_string(), &*displayname)?;
        } else {
            self.userid_displayname.remove(user_id.to_string())?;
        }

        Ok(())
    }

    /// Get a the avatar_url of a user.
    pub fn avatar_url(&self, user_id: &UserId) -> Result<Option<String>> {
        self.userid_avatarurl
            .get(user_id.to_string())?
            .map_or(Ok(None), |bytes| utils::string_from_bytes(&bytes).map(Some))
    }

    /// Sets a new avatar_url or removes it if avatar_url is None.
    pub fn set_avatar_url(&self, user_id: &UserId, avatar_url: Option<String>) -> Result<()> {
        if let Some(avatar_url) = avatar_url {
            self.userid_avatarurl
                .insert(user_id.to_string(), &*avatar_url)?;
        } else {
            self.userid_avatarurl.remove(user_id.to_string())?;
        }

        Ok(())
    }

    /// Adds a new device to a user.
    pub fn create_device(&self, user_id: &UserId, device_id: &DeviceId, token: &str) -> Result<()> {
        if !self.exists(user_id)? {
            return Err(Error::BadRequest(
                "tried to create device for nonexistent user",
            ));
        }

        let mut key = user_id.to_string().as_bytes().to_vec();
        key.push(0xff);
        key.extend_from_slice(device_id.as_bytes());

        self.userdeviceids.insert(key, &[])?;

        self.set_token(user_id, device_id, token)?;

        Ok(())
    }

    /// Returns an iterator over all device ids of this user.
    pub fn all_device_ids(&self, user_id: &UserId) -> impl Iterator<Item = Result<DeviceId>> {
        let mut prefix = user_id.to_string().as_bytes().to_vec();
        prefix.push(0xff);
        self.userdeviceids.scan_prefix(prefix).keys().map(|bytes| {
            Ok(utils::string_from_bytes(
                &*bytes?
                    .rsplit(|&b| b == 0xff)
                    .next()
                    .ok_or(Error::BadDatabase("userdeviceid is invalid"))?,
            )?)
        })
    }

    /// Replaces the access token of one device.
    pub fn set_token(&self, user_id: &UserId, device_id: &DeviceId, token: &str) -> Result<()> {
        let mut userdeviceid = user_id.to_string().as_bytes().to_vec();
        userdeviceid.push(0xff);
        userdeviceid.extend_from_slice(device_id.as_bytes());

        if self.userdeviceids.get(&userdeviceid)?.is_none() {
            return Err(Error::BadRequest(
                "Tried to set token for nonexistent device",
            ));
        }

        // Remove old token
        if let Some(old_token) = self.userdeviceid_token.get(&userdeviceid)? {
            self.token_userdeviceid.remove(old_token)?;
            // It will be removed from userdeviceid_token by the insert later
        }

        // Assign token to user device combination
        self.userdeviceid_token.insert(&userdeviceid, &*token)?;
        self.token_userdeviceid.insert(token, userdeviceid)?;

        Ok(())
    }

    pub fn add_one_time_key(
        &self,
        user_id: &UserId,
        device_id: &DeviceId,
        one_time_key_key: &AlgorithmAndDeviceId,
        one_time_key_value: &OneTimeKey,
    ) -> Result<()> {
        let mut key = user_id.to_string().as_bytes().to_vec();
        key.push(0xff);
        key.extend_from_slice(device_id.as_bytes());

        if self.userdeviceids.get(&key)?.is_none() {
            return Err(Error::BadRequest(
                "Tried to set token for nonexistent device",
            ));
        }

        key.push(0xff);
        // TODO: Use AlgorithmAndDeviceId::to_string when it's available (and update everything,
        // because there are no wrapping quotation marks anymore)
        key.extend_from_slice(&serde_json::to_string(one_time_key_key)?.as_bytes());

        self.onetimekeyid_onetimekeys
            .insert(&key, &*serde_json::to_string(&one_time_key_value)?)?;

        Ok(())
    }

    pub fn take_one_time_key(
        &self,
        user_id: &UserId,
        device_id: &DeviceId,
        key_algorithm: &KeyAlgorithm,
    ) -> Result<Option<(AlgorithmAndDeviceId, OneTimeKey)>> {
        let mut prefix = user_id.to_string().as_bytes().to_vec();
        prefix.push(0xff);
        prefix.extend_from_slice(device_id.as_bytes());
        prefix.push(0xff);
        prefix.push(b'"'); // Annoying quotation mark
        prefix.extend_from_slice(key_algorithm.to_string().as_bytes());
        prefix.push(b':');

        self.onetimekeyid_onetimekeys
            .scan_prefix(&prefix)
            .next()
            .map(|r| {
                let (key, value) = r?;
                Ok((
                    serde_json::from_slice(
                        &*key
                            .rsplit(|&b| b == 0xff)
                            .next()
                            .ok_or(Error::BadDatabase("onetimekeyid is invalid"))?,
                    )?,
                    serde_json::from_slice(&*value)?,
                ))
            })
            .transpose()
    }

    pub fn count_one_time_keys(
        &self,
        user_id: &UserId,
        device_id: &DeviceId,
    ) -> Result<BTreeMap<KeyAlgorithm, UInt>> {
        let mut userdeviceid = user_id.to_string().as_bytes().to_vec();
        userdeviceid.push(0xff);
        userdeviceid.extend_from_slice(device_id.as_bytes());

        let mut counts = BTreeMap::new();

        for algorithm in self
            .onetimekeyid_onetimekeys
            .scan_prefix(&userdeviceid)
            .keys()
            .map(|bytes| {
                Ok::<_, Error>(
                    serde_json::from_slice::<AlgorithmAndDeviceId>(
                        &*bytes?
                            .rsplit(|&b| b == 0xff)
                            .next()
                            .ok_or(Error::BadDatabase("onetimekeyid is invalid"))?,
                    )?
                    .0,
                )
            })
        {
            *counts.entry(algorithm?).or_default() += UInt::from(1_u32);
        }

        Ok(counts)
    }

    pub fn add_device_keys(
        &self,
        user_id: &UserId,
        device_id: &DeviceId,
        device_keys: &DeviceKeys,
    ) -> Result<()> {
        let mut userdeviceid = user_id.to_string().as_bytes().to_vec();
        userdeviceid.push(0xff);
        userdeviceid.extend_from_slice(device_id.as_bytes());

        self.userdeviceid_devicekeys
            .insert(&userdeviceid, &*serde_json::to_string(&device_keys)?)?;

        Ok(())
    }

    pub fn get_device_keys(
        &self,
        user_id: &UserId,
        device_id: &DeviceId,
    ) -> impl Iterator<Item = Result<DeviceKeys>> {
        let mut key = user_id.to_string().as_bytes().to_vec();
        key.push(0xff);
        key.extend_from_slice(device_id.as_bytes());

        self.userdeviceid_devicekeys
            .scan_prefix(key)
            .values()
            .map(|bytes| Ok(serde_json::from_slice(&bytes?)?))
    }

    pub fn all_device_keys(
        &self,
        user_id: &UserId,
    ) -> impl Iterator<Item = Result<(String, DeviceKeys)>> {
        let mut key = user_id.to_string().as_bytes().to_vec();
        key.push(0xff);

        self.userdeviceid_devicekeys.scan_prefix(key).map(|r| {
            let (key, value) = r?;
            Ok((
                utils::string_from_bytes(
                    key.rsplit(|&b| b == 0xff)
                        .next()
                        .ok_or(Error::BadDatabase("userdeviceid is invalid"))?,
                )?,
                serde_json::from_slice(&*value)?,
            ))
        })
    }

    pub fn add_to_device_event(
        &self,
        sender: &UserId,
        target_user_id: &UserId,
        target_device_id: &DeviceId,
        event_type: &EventType,
        content: serde_json::Value,
        globals: &super::globals::Globals,
    ) -> Result<()> {
        let mut key = target_user_id.to_string().as_bytes().to_vec();
        key.push(0xff);
        key.extend_from_slice(target_device_id.as_bytes());
        key.push(0xff);
        key.extend_from_slice(&globals.next_count()?.to_be_bytes());

        let mut json = serde_json::Map::new();
        json.insert("type".to_owned(), event_type.to_string().into());
        json.insert("sender".to_owned(), sender.to_string().into());
        json.insert("content".to_owned(), content);

        self.todeviceid_events
            .insert(&key, &*serde_json::to_string(&json)?)?;

        Ok(())
    }

    pub fn take_to_device_events(
        &self,
        user_id: &UserId,
        device_id: &DeviceId,
        max: usize,
    ) -> Result<Vec<EventJson<AnyToDeviceEvent>>> {
        let mut events = Vec::new();

        let mut prefix = user_id.to_string().as_bytes().to_vec();
        prefix.push(0xff);
        prefix.extend_from_slice(device_id.as_bytes());
        prefix.push(0xff);

        for result in self.todeviceid_events.scan_prefix(&prefix).take(max) {
            let (key, value) = result?;
            events.push(serde_json::from_slice(&*value)?);
            self.todeviceid_events.remove(key)?;
        }

        Ok(events)
    }
}
