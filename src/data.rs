use crate::{utils, Database};
use ruma_events::collections::all::Event;
use ruma_identifiers::{EventId, RoomId, UserId};
use std::convert::TryInto;

pub struct Data {
    hostname: String,
    db: Database,
}

impl Data {
    /// Load an existing database or create a new one.
    pub fn load_or_create(hostname: &str) -> Self {
        Self {
            hostname: hostname.to_owned(),
            db: Database::load_or_create(hostname),
        }
    }

    /// Get the hostname of the server.
    pub fn hostname(&self) -> &str {
        &self.hostname
    }

    /// Check if a user has an account by looking for an assigned password.
    pub fn user_exists(&self, user_id: &UserId) -> bool {
        self.db
            .userid_password
            .contains_key(user_id.to_string())
            .unwrap()
    }

    /// Create a new user account by assigning them a password.
    pub fn user_add(&self, user_id: &UserId, password: Option<String>) {
        self.db
            .userid_password
            .insert(user_id.to_string(), &*password.unwrap_or_default())
            .unwrap();
    }

    /// Find out which user an access token belongs to.
    pub fn user_from_token(&self, token: &str) -> Option<UserId> {
        self.db
            .token_userid
            .get(token)
            .unwrap()
            .and_then(|bytes| (*utils::string_from_bytes(&bytes)).try_into().ok())
    }

    /// Checks if the given password is equal to the one in the database.
    pub fn password_get(&self, user_id: &UserId) -> Option<String> {
        self.db
            .userid_password
            .get(user_id.to_string())
            .unwrap()
            .map(|bytes| utils::string_from_bytes(&bytes))
    }

    /// Add a new device to a user.
    pub fn device_add(&self, user_id: &UserId, device_id: &str) {
        if self
            .db
            .userid_deviceids
            .get_iter(&user_id.to_string().as_bytes())
            .filter_map(|item| item.ok())
            .map(|(_key, value)| value)
            .all(|device| device != device_id)
        {
            self.db
                .userid_deviceids
                .add(user_id.to_string().as_bytes(), device_id.into());
        }
    }

    /// Replace the access token of one device.
    pub fn token_replace(&self, user_id: &UserId, device_id: &String, token: String) {
        // Make sure the device id belongs to the user
        debug_assert!(self
            .db
            .userid_deviceids
            .get_iter(&user_id.to_string().as_bytes())
            .filter_map(|item| item.ok())
            .map(|(_key, value)| value)
            .any(|device| device == device_id.as_bytes())); // Does the user have that device?

        // Remove old token
        if let Some(old_token) = self.db.deviceid_token.get(device_id).unwrap() {
            self.db.token_userid.remove(old_token).unwrap();
            // It will be removed from deviceid_token by the insert later
        }

        // Assign token to device_id
        self.db.deviceid_token.insert(device_id, &*token).unwrap();

        // Assign token to user
        self.db
            .token_userid
            .insert(token, &*user_id.to_string())
            .unwrap();
    }

    /// Create a new room event.
    pub fn event_add(&self, room_id: &RoomId, event_id: &EventId, event: &Event) {
        let mut key = room_id.to_string().as_bytes().to_vec();
        key.extend_from_slice(event_id.to_string().as_bytes());
        self.db
            .roomid_eventid_event
            .insert(&key, &*serde_json::to_string(event).unwrap())
            .unwrap();
    }

    pub fn debug(&self) {
        self.db.debug();
    }
}
