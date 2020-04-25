use crate::{utils, Database, PduEvent};
use ruma_events::{collections::only::Event as EduEvent, EventJson, EventType};
use ruma_federation_api::RoomV3Pdu;
use ruma_identifiers::{EventId, RoomId, UserId};
use serde_json::json;
use std::{
    collections::HashMap,
    convert::{TryFrom, TryInto},
};

pub struct Data {
    hostname: String,
    reqwest_client: reqwest::Client,
    db: Database,
}

impl Data {
    /// Load an existing database or create a new one.
    pub fn load_or_create(hostname: &str) -> Self {
        let db = Database::load_or_create(hostname);
        Self {
            hostname: hostname.to_owned(),
            reqwest_client: reqwest::Client::new(),
            db,
        }
    }

    /// Get the hostname of the server.
    pub fn hostname(&self) -> &str {
        &self.hostname
    }

    /// Get the hostname of the server.
    pub fn reqwest_client(&self) -> &reqwest::Client {
        &self.reqwest_client
    }

    pub fn keypair(&self) -> &ruma_signatures::Ed25519KeyPair {
        &self.db.keypair
    }

    /// Check if a user has an account by looking for an assigned password.
    pub fn user_exists(&self, user_id: &UserId) -> bool {
        self.db
            .userid_password
            .contains_key(user_id.to_string())
            .unwrap()
    }

    /// Create a new user account by assigning them a password.
    pub fn user_add(&self, user_id: &UserId, hash: &str) {
        self.db
            .userid_password
            .insert(user_id.to_string(), hash)
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

    pub fn users_all(&self) -> Vec<UserId> {
        self.db
            .userid_password
            .iter()
            .keys()
            .map(|k| UserId::try_from(&*utils::string_from_bytes(&k.unwrap())).unwrap())
            .collect()
    }

    /// Gets password hash for given user id.
    pub fn password_hash_get(&self, user_id: &UserId) -> Option<String> {
        self.db
            .userid_password
            .get(user_id.to_string())
            .unwrap()
            .map(|bytes| utils::string_from_bytes(&bytes))
    }

    /// Removes a displayname.
    pub fn displayname_remove(&self, user_id: &UserId) {
        self.db
            .userid_displayname
            .remove(user_id.to_string())
            .unwrap();
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
        self.db
            .userid_avatarurl
            .remove(user_id.to_string())
            .unwrap();
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
        let mut key = user_id.to_string().as_bytes().to_vec();
        key.push(0xff);
        key.extend_from_slice(device_id.as_bytes());
        if let Some(old_token) = self.db.userdeviceid_token.get(&key).unwrap() {
            self.db.token_userid.remove(old_token).unwrap();
            // It will be removed from deviceid_token by the insert later
        }

        // Assign token to device_id
        self.db.userdeviceid_token.insert(key, &*token).unwrap();

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
        self.db.userid_inviteroomids.remove_value(
            user_id.to_string().as_bytes(),
            room_id.to_string().as_bytes(),
        );
        self.db.userid_leftroomids.remove_value(
            user_id.to_string().as_bytes(),
            room_id.to_string().as_bytes().into(),
        );

        self.pdu_append(
            room_id.clone(),
            user_id.clone(),
            EventType::RoomMember,
            json!({"membership": "join"}),
            None,
            Some(user_id.to_string()),
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
        prefix.push(0xff); // Add delimiter so we don't find rooms starting with the same id

        if let Some((key, _)) = self.db.pduid_pdu.get_gt(&prefix).unwrap() {
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

    pub fn room_state(&self, room_id: &RoomId) -> HashMap<(EventType, String), PduEvent> {
        let mut hashmap = HashMap::new();
        for pdu in self
            .db
            .roomstateid_pdu
            .scan_prefix(&room_id.to_string().as_bytes())
            .values()
            .map(|value| serde_json::from_slice::<PduEvent>(&value.unwrap()).unwrap())
        {
            hashmap.insert(
                (
                    pdu.kind.clone(),
                    pdu.state_key
                        .clone()
                        .expect("state events have a state key"),
                ),
                pdu,
            );
        }
        hashmap
    }

    pub fn room_leave(&self, sender: &UserId, room_id: &RoomId, user_id: &UserId) {
        self.pdu_append(
            room_id.clone(),
            sender.clone(),
            EventType::RoomMember,
            json!({"membership": "leave"}),
            None,
            Some(user_id.to_string()),
        );
        self.db.userid_inviteroomids.remove_value(
            user_id.to_string().as_bytes(),
            room_id.to_string().as_bytes().into(),
        );
        self.db.userid_roomids.remove_value(
            user_id.to_string().as_bytes(),
            room_id.to_string().as_bytes().into(),
        );
        self.db.roomid_userids.remove_value(
            room_id.to_string().as_bytes(),
            user_id.to_string().as_bytes().into(),
        );
        self.db.userid_leftroomids.add(
            user_id.to_string().as_bytes(),
            room_id.to_string().as_bytes().into(),
        );
    }

    pub fn room_invite(&self, sender: &UserId, room_id: &RoomId, user_id: &UserId) {
        self.pdu_append(
            room_id.clone(),
            sender.clone(),
            EventType::RoomMember,
            json!({"membership": "invite"}),
            None,
            Some(user_id.to_string()),
        );
        self.db.userid_inviteroomids.add(
            user_id.to_string().as_bytes(),
            room_id.to_string().as_bytes().into(),
        );
        self.db.roomid_userids.add(
            room_id.to_string().as_bytes(),
            user_id.to_string().as_bytes().into(),
        );
    }

    pub fn rooms_invited(&self, user_id: &UserId) -> Vec<RoomId> {
        self.db
            .userid_inviteroomids
            .get_iter(&user_id.to_string().as_bytes())
            .values()
            .map(|key| RoomId::try_from(&*utils::string_from_bytes(&key.unwrap())).unwrap())
            .collect()
    }

    pub fn rooms_left(&self, user_id: &UserId) -> Vec<RoomId> {
        self.db
            .userid_leftroomids
            .get_iter(&user_id.to_string().as_bytes())
            .values()
            .map(|key| RoomId::try_from(&*utils::string_from_bytes(&key.unwrap())).unwrap())
            .collect()
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
                        .pduid_pdu
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
            origin_server_ts: utils::millis_since_unix_epoch().try_into().unwrap(),
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

        let mut pdu_json = serde_json::to_value(&pdu).unwrap();
        ruma_signatures::hash_and_sign_event(self.hostname(), self.keypair(), &mut pdu_json)
            .unwrap();

        self.pdu_leaves_replace(&room_id, &pdu.event_id);

        // The new value will need a new index. We store the last used index in 'n'
        // The count will go up regardless of the room_id
        // This is also the next_batch/since value
        // Increment the last index and use that
        let index = utils::u64_from_bytes(
            &self
                .db
                .pduid_pdu
                .update_and_fetch(b"n", utils::increment)
                .unwrap()
                .unwrap(),
        );

        let mut pdu_id = vec![b'd'];
        pdu_id.extend_from_slice(room_id.to_string().as_bytes());

        pdu_id.push(0xff); // Add delimiter so we don't find rooms starting with the same id
        pdu_id.extend_from_slice(&index.to_be_bytes());

        self.db
            .pduid_pdu
            .insert(&pdu_id, &*pdu_json.to_string())
            .unwrap();

        self.db
            .eventid_pduid
            .insert(pdu.event_id.to_string(), pdu_id.clone())
            .unwrap();

        if let Some(state_key) = pdu.state_key {
            let mut key = room_id.to_string().as_bytes().to_vec();
            key.push(0xff);
            key.extend_from_slice(pdu.kind.to_string().as_bytes());
            key.push(0xff);
            key.extend_from_slice(state_key.to_string().as_bytes());
            self.db
                .roomstateid_pdu
                .insert(key, &*pdu_json.to_string())
                .unwrap();
        }

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
                .pduid_pdu
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
        prefix.push(0xff); // Add delimiter so we don't find rooms starting with the same id

        let mut current = prefix.clone();
        current.extend_from_slice(&since.to_be_bytes());

        while let Some((key, value)) = self.db.pduid_pdu.get_gt(&current).unwrap() {
            if key.starts_with(&prefix) {
                current = key.to_vec();
                pdus.push(serde_json::from_slice(&value).expect("pdu in db is valid"));
            } else {
                break;
            }
        }

        pdus
    }

    pub fn roomlatest_update(&self, user_id: &UserId, room_id: &RoomId, event: EduEvent) {
        let mut prefix = room_id.to_string().as_bytes().to_vec();
        prefix.push(0xff);

        // Start with last
        if let Some(mut current) = self
            .db
            .roomlatestid_roomlatest
            .scan_prefix(&prefix)
            .keys()
            .next_back()
            .map(|c| c.unwrap())
        {
            // Remove old marker (There should at most one)
            loop {
                if !current.starts_with(&prefix) {
                    // We're in another room
                    break;
                }
                if current.rsplitn(2, |&b| b == 0xff).next().unwrap()
                    == user_id.to_string().as_bytes()
                {
                    // This is the old room_latest
                    self.db.roomlatestid_roomlatest.remove(current).unwrap();
                    break;
                }
                // Else, try the event before that
                if let Some((k, _)) = self.db.roomlatestid_roomlatest.get_lt(current).unwrap() {
                    current = k;
                } else {
                    break;
                }
            }
        }

        // Increment the last index and use that
        let index = utils::u64_from_bytes(
            &self
                .db
                .pduid_pdu
                .update_and_fetch(b"n", utils::increment)
                .unwrap()
                .unwrap(),
        );

        let mut room_latest_id = prefix;
        room_latest_id.extend_from_slice(&index.to_be_bytes());
        room_latest_id.push(0xff);
        room_latest_id.extend_from_slice(&user_id.to_string().as_bytes());

        self.db
            .roomlatestid_roomlatest
            .insert(room_latest_id, &*serde_json::to_string(&event).unwrap())
            .unwrap();
    }

    /// Returns a vector of the most recent read_receipts in a room that happened after the event with id `since`.
    pub fn roomlatests_since(&self, room_id: &RoomId, since: u64) -> Vec<EventJson<EduEvent>> {
        let mut room_latests = Vec::new();

        let mut prefix = room_id.to_string().as_bytes().to_vec();
        prefix.push(0xff);

        let mut current = prefix.clone();
        current.extend_from_slice(&since.to_be_bytes());

        while let Some((key, value)) = self.db.roomlatestid_roomlatest.get_gt(&current).unwrap() {
            if key.starts_with(&prefix) {
                current = key.to_vec();
                room_latests.push(
                    serde_json::from_slice::<EventJson<EduEvent>>(&value)
                        .expect("room_latest in db is valid")
                        .deserialize()
                        .expect("room_latest in db is valid")
                        .into(),
                );
            } else {
                break;
            }
        }

        room_latests
    }

    pub fn roomactive_add(&self, event: EduEvent, room_id: &RoomId, timeout: u64) {
        let mut prefix = room_id.to_string().as_bytes().to_vec();
        prefix.push(0xff);

        let mut current = prefix.clone();

        while let Some((key, _)) = self.db.roomactiveid_roomactive.get_gt(&current).unwrap() {
            if key.starts_with(&prefix)
                && utils::u64_from_bytes(key.split(|&c| c == 0xff).nth(1).unwrap())
                    > utils::millis_since_unix_epoch().try_into().unwrap()
            {
                current = key.to_vec();
                self.db.roomactiveid_roomactive.remove(&current).unwrap();
            } else {
                break;
            }
        }

        // Increment the last index and use that
        let index = utils::u64_from_bytes(
            &self
                .db
                .pduid_pdu
                .update_and_fetch(b"n", utils::increment)
                .unwrap()
                .unwrap(),
        );

        let mut room_active_id = prefix;
        room_active_id.extend_from_slice(&timeout.to_be_bytes());
        room_active_id.push(0xff);
        room_active_id.extend_from_slice(&index.to_be_bytes());

        self.db
            .roomactiveid_roomactive
            .insert(room_active_id, &*serde_json::to_string(&event).unwrap())
            .unwrap();
    }

    pub fn roomactive_remove(&self, event: EduEvent, room_id: &RoomId) {
        let mut prefix = room_id.to_string().as_bytes().to_vec();
        prefix.push(0xff);

        let mut current = prefix.clone();

        let json = serde_json::to_string(&event).unwrap();

        while let Some((key, value)) = self.db.roomactiveid_roomactive.get_gt(&current).unwrap() {
            if key.starts_with(&prefix) {
                current = key.to_vec();
                if value == json.as_bytes() {
                    self.db.roomactiveid_roomactive.remove(&current).unwrap();
                    break;
                }
            } else {
                break;
            }
        }
    }

    /// Returns a vector of the most recent read_receipts in a room that happened after the event with id `since`.
    pub fn roomactives_in(&self, room_id: &RoomId) -> Vec<EventJson<EduEvent>> {
        let mut room_actives = Vec::new();

        let mut prefix = room_id.to_string().as_bytes().to_vec();
        prefix.push(0xff);

        let mut current = prefix.clone();
        current.extend_from_slice(&utils::millis_since_unix_epoch().to_be_bytes());

        while let Some((key, value)) = self.db.roomactiveid_roomactive.get_gt(&current).unwrap() {
            if key.starts_with(&prefix) {
                current = key.to_vec();
                room_actives.push(
                    serde_json::from_slice::<EventJson<EduEvent>>(&value)
                        .expect("room_active in db is valid")
                        .deserialize()
                        .expect("room_active in db is valid")
                        .into(),
                );
            } else {
                break;
            }
        }

        if room_actives.is_empty() {
            return vec![EduEvent::Typing(ruma_events::typing::TypingEvent {
                content: ruma_events::typing::TypingEventContent {
                    user_ids: Vec::new(),
                },
                room_id: None, // None because it can be inferred
            })
            .into()];
        } else {
            room_actives
        }
    }

    pub fn debug(&self) {
        self.db.debug();
    }
}
