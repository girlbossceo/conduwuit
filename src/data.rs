use crate::utils;
use directories::ProjectDirs;
use log::debug;
use ruma_events::collections::all::Event;
use ruma_identifiers::{EventId, RoomId, UserId};
use std::convert::TryInto;

const USERID_PASSWORD: &str = "userid_password";
const USERID_DEVICEIDS: &str = "userid_deviceids";
const DEVICEID_TOKEN: &str = "deviceid_token";
const TOKEN_USERID: &str = "token_userid";

pub struct Data(sled::Db);

impl Data {
    /// Load an existing database or create a new one.
    pub fn load_or_create() -> Self {
        Data(
            sled::open(
                ProjectDirs::from("xyz", "koesters", "matrixserver")
                    .unwrap()
                    .data_dir(),
            )
            .unwrap(),
        )
    }

    /// Set the hostname of the server. Warning: Hostname changes will likely break things.
    pub fn set_hostname(&self, hostname: &str) {
        self.0.insert("hostname", hostname).unwrap();
    }

    /// Get the hostname of the server.
    pub fn hostname(&self) -> String {
        utils::bytes_to_string(&self.0.get("hostname").unwrap().unwrap())
    }

    /// Check if a user has an account by looking for an assigned password.
    pub fn user_exists(&self, user_id: &UserId) -> bool {
        self.0
            .open_tree(USERID_PASSWORD)
            .unwrap()
            .contains_key(user_id.to_string())
            .unwrap()
    }

    /// Create a new user account by assigning them a password.
    pub fn user_add(&self, user_id: &UserId, password: Option<String>) {
        self.0
            .open_tree(USERID_PASSWORD)
            .unwrap()
            .insert(user_id.to_string(), &*password.unwrap_or_default())
            .unwrap();
    }

    /// Find out which user an access token belongs to.
    pub fn user_from_token(&self, token: &str) -> Option<UserId> {
        self.0
            .open_tree(TOKEN_USERID)
            .unwrap()
            .get(token)
            .unwrap()
            .and_then(|bytes| (*utils::bytes_to_string(&bytes)).try_into().ok())
    }

    /// Checks if the given password is equal to the one in the database.
    pub fn password_get(&self, user_id: &UserId) -> Option<String> {
        self.0
            .open_tree(USERID_PASSWORD)
            .unwrap()
            .get(user_id.to_string())
            .unwrap()
            .map(|bytes| utils::bytes_to_string(&bytes))
    }

    /// Add a new device to a user.
    pub fn device_add(&self, user_id: &UserId, device_id: &str) {
        self.0
            .open_tree(USERID_DEVICEIDS)
            .unwrap()
            .insert(user_id.to_string(), device_id)
            .unwrap();
    }

    /// Replace the access token of one device.
    pub fn token_replace(&self, user_id: &UserId, device_id: &String, token: String) {
        // Make sure the device id belongs to the user
        debug_assert!(self
            .0
            .open_tree(USERID_DEVICEIDS)
            .unwrap()
            .get(&user_id.to_string()) // Does the user exist?
            .unwrap()
            .map(|bytes| utils::bytes_to_vec(&bytes))
            .filter(|devices| devices.contains(device_id)) // Does the user have that device?
            .is_some());

        // Remove old token
        if let Some(old_token) = self
            .0
            .open_tree(DEVICEID_TOKEN)
            .unwrap()
            .get(device_id)
            .unwrap()
        {
            self.0
                .open_tree(TOKEN_USERID)
                .unwrap()
                .remove(old_token)
                .unwrap();
            // It will be removed from DEVICEID_TOKEN by the insert later
        }

        // Assign token to device_id
        self.0
            .open_tree(DEVICEID_TOKEN)
            .unwrap()
            .insert(device_id, &*token)
            .unwrap();

        // Assign token to user
        self.0
            .open_tree(TOKEN_USERID)
            .unwrap()
            .insert(token, &*user_id.to_string())
            .unwrap();
    }

    /// Create a new room event.
    pub fn event_add(&self, event: &Event, room_id: &RoomId, event_id: &EventId) {
        debug!("{}", serde_json::to_string(event).unwrap());
        todo!();
    }
}
