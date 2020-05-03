use crate::{utils, Error, Result};
use ruma_identifiers::UserId;
use std::convert::TryFrom;

pub struct Users {
    pub(super) userid_password: sled::Tree,
    pub(super) userid_displayname: sled::Tree,
    pub(super) userid_avatarurl: sled::Tree,
    pub(super) userdeviceid: sled::Tree,
    pub(super) userdeviceid_token: sled::Tree,
    pub(super) token_userid: sled::Tree,
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
    pub fn find_from_token(&self, token: &str) -> Result<Option<UserId>> {
        self.token_userid.get(token)?.map_or(Ok(None), |bytes| {
            utils::string_from_bytes(&bytes)
                .and_then(|string| Ok(UserId::try_from(string)?))
                .map(Some)
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
        /* TODO:
        for room_id in self.rooms_joined(user_id) {
            self.pdu_append(
                room_id.clone(),
                user_id.clone(),
                EventType::RoomMember,
                json!({"membership": "join", "displayname": displayname}),
                None,
                Some(user_id.to_string()),
            );
        }
        */
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
    pub fn create_device(&self, user_id: &UserId, device_id: &str, token: &str) -> Result<()> {
        if !self.exists(user_id)? {
            return Err(Error::BadRequest(
                "tried to create device for nonexistent user",
            ));
        }

        let mut key = user_id.to_string().as_bytes().to_vec();
        key.push(0xff);
        key.extend_from_slice(device_id.as_bytes());

        self.userdeviceid.insert(key, &[])?;

        self.set_token(user_id, device_id, token)?;

        Ok(())
    }

    /// Replaces the access token of one device.
    pub fn set_token(&self, user_id: &UserId, device_id: &str, token: &str) -> Result<()> {
        let mut key = user_id.to_string().as_bytes().to_vec();
        key.push(0xff);
        key.extend_from_slice(device_id.as_bytes());

        if self.userdeviceid.get(&key)?.is_none() {
            return Err(Error::BadRequest(
                "Tried to set token for nonexistent device",
            ));
        }

        // Remove old token
        if let Some(old_token) = self.userdeviceid_token.get(&key)? {
            self.token_userid.remove(old_token)?;
            // It will be removed from userdeviceid_token by the insert later
        }

        // Assign token to device_id
        self.userdeviceid_token.insert(key, &*token)?;

        // Assign token to user
        self.token_userid.insert(token, &*user_id.to_string())?;

        Ok(())
    }
}
