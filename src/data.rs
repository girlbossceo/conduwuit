use crate::{utils, Database, PduEvent};
use ruma_events::EventType;
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

    /// Removes a displayname.
    pub fn displayname_remove(&self, user_id: &UserId) {
        self.db.userid_displayname.remove(user_id).unwrap();
    }

    /// Set a new displayname.
    pub fn displayname_set(&self, user_id: &UserId, displayname: Option<String>) {
        self.db
            .userid_displayname
            .insert(user_id.to_string(), &*displayname.unwrap_or_default())
            .unwrap();
    }

    /// Get a the displayname of a user.
    pub fn displayname_get(&self, user_id: &UserId) -> Option<String> {
        self.db
            .userid_displayname
            .get(user_id.to_string())
            .unwrap()
            .map(|bytes| utils::string_from_bytes(&bytes))
    }

    /// Removes a avatar_url.
    pub fn avatar_url_remove(&self, user_id: &UserId) {
        self.db.userid_avatarurl.remove(user_id).unwrap();
    }

    /// Set a new avatar_url.
    pub fn avatar_url_set(&self, user_id: &UserId, avatar_url: String) {
        self.db
            .userid_avatarurl
            .insert(user_id.to_string(), &*avatar_url)
            .unwrap();
    }

    /// Get a the avatar_url of a user.
    pub fn avatar_url_get(&self, user_id: &UserId) -> Option<String> {
        self.db
            .userid_avatarurl
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

    pub fn room_join(&self, room_id: &RoomId, user_id: &UserId) -> bool {
        if !self.room_exists(room_id) {
            return false;
        }

        self.db.userid_roomids.add(
            user_id.to_string().as_bytes(),
            room_id.to_string().as_bytes().into(),
        );
        self.db.roomid_userids.add(
            room_id.to_string().as_bytes(),
            user_id.to_string().as_bytes().into(),
        );

        true
    }

    pub fn rooms_joined(&self, user_id: &UserId) -> Vec<RoomId> {
        self.db
            .userid_roomids
            .get_iter(user_id.to_string().as_bytes())
            .values()
            .map(|room_id| {
                RoomId::try_from(&*utils::string_from_bytes(&room_id.unwrap()))
                    .expect("user joined valid room ids")
            })
            .collect()
    }

    /// Check if a room exists by looking for PDUs in that room.
    pub fn room_exists(&self, room_id: &RoomId) -> bool {
        // Create the first part of the full pdu id
        let mut prefix = vec![b'd'];
        prefix.extend_from_slice(room_id.to_string().as_bytes());
        prefix.push(b'#'); // Add delimiter so we don't find rooms starting with the same id

        if let Some((key, _)) = self.db.pduid_pdus.get_gt(&prefix).unwrap() {
            if key.starts_with(&prefix) {
                true
            } else {
                false
            }
        } else {
            false
        }
    }

    pub fn rooms_all(&self) -> Vec<RoomId> {
        let mut room_ids = self
            .db
            .roomid_pduleaves
            .iter_all()
            .keys()
            .map(|key| {
                RoomId::try_from(&*utils::string_from_bytes(
                    &key.unwrap()
                        .iter()
                        .skip(1) // skip "d"
                        .copied()
                        .take_while(|&x| x != 0xff) // until delimiter
                        .collect::<Vec<_>>(),
                ))
                .unwrap()
            })
            .collect::<Vec<_>>();
        room_ids.dedup();
        room_ids
    }

    pub fn room_users(&self, room_id: &RoomId) -> u32 {
        self.db
            .roomid_userids
            .get_iter(room_id.to_string().as_bytes())
            .count() as u32
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

    pub fn pdu_leaves_get(&self, room_id: &RoomId) -> Vec<EventId> {
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

        event_ids
    }

    pub fn pdu_leaves_replace(&self, room_id: &RoomId, event_id: &EventId) {
        self.db
            .roomid_pduleaves
            .clear(room_id.to_string().as_bytes());

        self.db.roomid_pduleaves.add(
            &room_id.to_string().as_bytes(),
            (*event_id.to_string()).into(),
        );
    }

    /// Add a persisted data unit from this homeserver
    pub fn pdu_append(
        &self,
        room_id: RoomId,
        sender: UserId,
        event_type: EventType,
        content: serde_json::Value,
        unsigned: Option<serde_json::Map<String, serde_json::Value>>,
        state_key: Option<String>,
    ) -> EventId {
        // prev_events are the leaves of the current graph. This method removes all leaves from the
        // room and replaces them with our event
        // TODO: Make sure this isn't called twice in parallel
        let prev_events = self.pdu_leaves_get(&room_id);

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

        let mut pdu = PduEvent {
            event_id: EventId::try_from("$thiswillbefilledinlater").unwrap(),
            room_id: room_id.clone(),
            sender: sender.clone(),
            origin: self.hostname.clone(),
            origin_server_ts: utils::millis_since_unix_epoch(),
            kind: event_type,
            content,
            state_key,
            prev_events,
            depth: depth.try_into().unwrap(),
            auth_events: Vec::new(),
            redacts: None,
            unsigned: unsigned.unwrap_or_default(),
            hashes: ruma_federation_api::EventHash {
                sha256: "aaa".to_owned(),
            },
            signatures: HashMap::new(),
        };

        // Generate event id
        pdu.event_id = EventId::try_from(&*format!(
            "${}",
            ruma_signatures::reference_hash(&serde_json::to_value(&pdu).unwrap())
                .expect("ruma can calculate reference hashes")
        ))
        .expect("ruma's reference hashes are correct");

        self.pdu_leaves_replace(&room_id, &pdu.event_id);

        // The new value will need a new index. We store the last used index in 'n'
        // The count will go up regardless of the room_id
        // This is also the next_batch/since value
        let count_key: Vec<u8> = vec![b'n'];

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
        pdu_id.extend_from_slice(&index.to_be_bytes());

        self.db
            .pduid_pdus
            .insert(&pdu_id, &*serde_json::to_string(&pdu).unwrap())
            .unwrap();

        self.db
            .eventid_pduid
            .insert(pdu.event_id.to_string(), pdu_id.clone())
            .unwrap();

        pdu.event_id
    }

    /// Returns a vector of all PDUs in a room.
    pub fn pdus_all(&self, room_id: &RoomId) -> Vec<PduEvent> {
        self.pdus_since(room_id, 0)
    }

    pub fn last_pdu_index(&self) -> u64 {
        let count_key: Vec<u8> = vec![b'n'];
        utils::u64_from_bytes(
            &self
                .db
                .pduid_pdus
                .get(&count_key)
                .unwrap()
                .unwrap_or_else(|| (&0_u64.to_be_bytes()).into()),
        )
    }

    /// Returns a vector of all events in a room that happened after the event with id `since`.
    pub fn pdus_since(&self, room_id: &RoomId, since: u64) -> Vec<PduEvent> {
        let mut pdus = Vec::new();

        // Create the first part of the full pdu id
        let mut prefix = vec![b'd'];
        prefix.extend_from_slice(room_id.to_string().as_bytes());
        prefix.push(b'#'); // Add delimiter so we don't find rooms starting with the same id

        let mut current = prefix.clone();
        current.extend_from_slice(&since.to_be_bytes());

        while let Some((key, value)) = self.db.pduid_pdus.get_gt(&current).unwrap() {
            if key.starts_with(&prefix) {
                current = key.to_vec();
                pdus.push(serde_json::from_slice(&value).expect("pdu in db is valid"));
            } else {
                break;
            }
        }

        pdus
    }

    pub fn debug(&self) {
        self.db.debug();
    }
}
