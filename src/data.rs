use crate::{utils, Database, PduEvent};
use log::debug;
use ruma_events::{room::message::MessageEvent, EventType};
use ruma_federation_api::RoomV3Pdu;
use ruma_identifiers::{EventId, RoomId, UserId};
use std::{
    collections::HashMap,
    convert::{TryFrom, TryInto},
};

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

    pub fn pdu_get(&self, event_id: &EventId) -> Option<RoomV3Pdu> {
        self.db
            .eventid_pduid
            .get(event_id.to_string().as_bytes())
            .unwrap()
            .map(|pdu_id| {
                serde_json::from_slice(
                    &self
                        .db
                        .pduid_pdus
                        .get(pdu_id)
                        .unwrap()
                        .expect("eventid_pduid in db is valid"),
                )
                .expect("pdu is valid")
            })
    }

    // TODO: Make sure this isn't called twice in parallel
    pub fn pdu_leaves_replace(&self, room_id: &RoomId, event_id: &EventId) -> Vec<EventId> {
        let event_ids = self
            .db
            .roomid_pduleaves
            .get_iter(room_id.to_string().as_bytes())
            .values()
            .map(|pdu_id| {
                EventId::try_from(&*utils::string_from_bytes(&pdu_id.unwrap()))
                    .expect("pdu leaves are valid event ids")
            })
            .collect();

        self.db
            .roomid_pduleaves
            .clear(room_id.to_string().as_bytes());

        self.db.roomid_pduleaves.add(
            &room_id.to_string().as_bytes(),
            (*event_id.to_string()).into(),
        );

        event_ids
    }

    /// Add a persisted data unit from this homeserver
    pub fn pdu_append_message(&self, event_id: &EventId, room_id: &RoomId, event: MessageEvent) {
        // prev_events are the leaves of the current graph. This method removes all leaves from the
        // room and replaces them with our event
        let prev_events = self.pdu_leaves_replace(room_id, event_id);

        // Our depth is the maximum depth of prev_events + 1
        let depth = prev_events
            .iter()
            .map(|event_id| {
                self.pdu_get(event_id)
                    .expect("pdu in prev_events is valid")
                    .depth
                    .into()
            })
            .max()
            .unwrap_or(0_u64)
            + 1;

        let pdu = PduEvent {
            event_id: event_id.clone(),
            room_id: room_id.clone(),
            sender: event.sender,
            origin: self.hostname.clone(),
            origin_server_ts: event.origin_server_ts,
            kind: EventType::RoomMessage,
            content: serde_json::to_value(event.content).unwrap(),
            state_key: None,
            prev_events,
            depth: depth.try_into().unwrap(),
            auth_events: Vec::new(),
            redacts: None,
            unsigned: Default::default(),
            hashes: ruma_federation_api::EventHash {
                sha256: "aaa".to_owned(),
            },
            signatures: HashMap::new(),
        };

        // The new value will need a new index. We store the last used index in 'n' + id
        let mut count_key: Vec<u8> = vec![b'n'];
        count_key.extend_from_slice(&room_id.to_string().as_bytes());

        // Increment the last index and use that
        let index = utils::u64_from_bytes(
            &self
                .db
                .pduid_pdus
                .update_and_fetch(&count_key, utils::increment)
                .unwrap()
                .unwrap(),
        );

        let mut pdu_id = vec![b'd'];
        pdu_id.extend_from_slice(room_id.to_string().as_bytes());

        pdu_id.push(b'#'); // Add delimiter so we don't find rooms starting with the same id
        pdu_id.extend_from_slice(index.to_string().as_bytes());

        self.db
            .pduid_pdus
            .insert(&pdu_id, &*serde_json::to_string(&pdu).unwrap())
            .unwrap();

        self.db
            .eventid_pduid
            .insert(event_id.to_string(), pdu_id.clone())
            .unwrap();
    }

    /// Returns a vector of all PDUs.
    pub fn pdus_all(&self) -> Vec<PduEvent> {
        self.pdus_since(
            self.db
                .eventid_pduid
                .iter()
                .values()
                .next()
                .unwrap()
                .map(|key| utils::string_from_bytes(&key))
                .expect("there should be at least one pdu"),
        )
    }

    /// Returns a vector of all events that happened after the event with id `since`.
    pub fn pdus_since(&self, since: String) -> Vec<PduEvent> {
        let mut pdus = Vec::new();

        if let Some(room_id) = since.rsplitn(2, '#').nth(1) {
            let mut current = since.clone();

            while let Some((key, value)) = self.db.pduid_pdus.get_gt(current).unwrap() {
                if key.starts_with(&room_id.to_string().as_bytes()) {
                    current = utils::string_from_bytes(&key);
                } else {
                    break;
                }
                pdus.push(serde_json::from_slice(&value).expect("pdu is valid"));
            }
        } else {
            debug!("event at `since` not found");
        }
        pdus
    }

    pub fn debug(&self) {
        self.db.debug();
    }
}
