mod edus;

pub use edus::RoomEdus;

use crate::{utils, Error, PduEvent, Result};
use ruma_events::{room::power_levels::PowerLevelsEventContent, EventJson, EventType};
use ruma_identifiers::{EventId, RoomId, UserId};
use serde_json::json;
use std::{
    collections::HashMap,
    convert::{TryFrom, TryInto},
    mem,
};

pub struct Rooms {
    pub edus: edus::RoomEdus,
    pub(super) pduid_pdu: sled::Tree, // PduId = RoomId + Count
    pub(super) eventid_pduid: sled::Tree,
    pub(super) roomid_pduleaves: sled::Tree,
    pub(super) roomstateid_pdu: sled::Tree, // RoomStateId = Room + StateType + StateKey

    pub(super) userroomid_joined: sled::Tree,
    pub(super) roomuserid_joined: sled::Tree,
    pub(super) userroomid_invited: sled::Tree,
    pub(super) roomuserid_invited: sled::Tree,
    pub(super) userroomid_left: sled::Tree,
}

impl Rooms {
    /// Checks if a room exists.
    pub fn exists(&self, room_id: &RoomId) -> Result<bool> {
        let mut prefix = room_id.to_string().as_bytes().to_vec();
        prefix.push(0xff);

        // Look for PDUs in that room.
        Ok(self
            .pduid_pdu
            .get_gt(&prefix)?
            .filter(|(k, _)| k.starts_with(&prefix))
            .is_some())
    }

    // TODO: Remove and replace with public room dir
    /// Returns a vector over all rooms.
    pub fn all_rooms(&self) -> Vec<RoomId> {
        let mut room_ids = self
            .roomid_pduleaves
            .iter()
            .keys()
            .map(|key| {
                RoomId::try_from(
                    &*utils::string_from_bytes(
                        &key.unwrap()
                            .iter()
                            .copied()
                            .take_while(|&x| x != 0xff) // until delimiter
                            .collect::<Vec<_>>(),
                    )
                    .unwrap(),
                )
                .unwrap()
            })
            .collect::<Vec<_>>();
        room_ids.dedup();
        room_ids
    }

    /// Returns the full room state.
    pub fn room_state(&self, room_id: &RoomId) -> Result<HashMap<(EventType, String), PduEvent>> {
        let mut hashmap = HashMap::new();
        for pdu in self
            .roomstateid_pdu
            .scan_prefix(&room_id.to_string().as_bytes())
            .values()
            .map(|value| Ok::<_, Error>(serde_json::from_slice::<PduEvent>(&value?)?))
        {
            let pdu = pdu?;
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
        Ok(hashmap)
    }

    /// Returns the `count` of this pdu's id.
    pub fn get_pdu_count(&self, event_id: &EventId) -> Result<Option<u64>> {
        Ok(self
            .eventid_pduid
            .get(event_id.to_string().as_bytes())?
            .map(|pdu_id| {
                utils::u64_from_bytes(&pdu_id[pdu_id.len() - mem::size_of::<u64>()..pdu_id.len()])
            }))
    }

    /// Returns the json of a pdu.
    pub fn get_pdu_json(&self, event_id: &EventId) -> Result<Option<serde_json::Value>> {
        self.eventid_pduid
            .get(event_id.to_string().as_bytes())?
            .map_or(Ok(None), |pdu_id| {
                Ok(serde_json::from_slice(
                    &self.pduid_pdu.get(pdu_id)?.ok_or(Error::BadDatabase(
                        "eventid_pduid points to nonexistent pdu",
                    ))?,
                )?)
                .map(Some)
            })
    }

    /// Returns the leaf pdus of a room.
    pub fn get_pdu_leaves(&self, room_id: &RoomId) -> Result<Vec<EventId>> {
        let mut prefix = room_id.to_string().as_bytes().to_vec();
        prefix.push(0xff);

        let mut events = Vec::new();

        for event in self
            .roomid_pduleaves
            .scan_prefix(prefix)
            .values()
            .map(|bytes| Ok::<_, Error>(EventId::try_from(&*utils::string_from_bytes(&bytes?)?)?))
        {
            events.push(event?);
        }

        Ok(events)
    }

    /// Replace the leaves of a room with a new event.
    pub fn replace_pdu_leaves(&self, room_id: &RoomId, event_id: &EventId) -> Result<()> {
        let mut prefix = room_id.to_string().as_bytes().to_vec();
        prefix.push(0xff);

        for key in self.roomid_pduleaves.scan_prefix(&prefix).keys() {
            self.roomid_pduleaves.remove(key?)?;
        }

        prefix.extend_from_slice(event_id.to_string().as_bytes());
        self.roomid_pduleaves
            .insert(&prefix, &*event_id.to_string())?;

        Ok(())
    }

    /// Creates a new persisted data unit and adds it to a room.
    pub fn append_pdu(
        &self,
        room_id: RoomId,
        sender: UserId,
        event_type: EventType,
        content: serde_json::Value,
        unsigned: Option<serde_json::Map<String, serde_json::Value>>,
        state_key: Option<String>,
        globals: &super::globals::Globals,
    ) -> Result<EventId> {
        // Is the event authorized?
        if state_key.is_some() {
            if let Some(pdu) = self
                .room_state(&room_id)?
                .get(&(EventType::RoomPowerLevels, "".to_owned()))
            {
                let power_levels = serde_json::from_value::<EventJson<PowerLevelsEventContent>>(
                    pdu.content.clone(),
                )?
                .deserialize()?;

                match event_type {
                    EventType::RoomMember => {
                        // Member events are okay for now (TODO)
                    }
                    _ if power_levels
                        .users
                        .get(&sender)
                        .unwrap_or(&power_levels.users_default)
                        <= &0.into() =>
                    {
                        // Not authorized
                        return Err(Error::BadRequest("event not authorized"));
                    }
                    // User has sufficient power
                    _ => {}
                }
            }
        }

        // prev_events are the leaves of the current graph. This method removes all leaves from the
        // room and replaces them with our event
        // TODO: Make sure this isn't called twice in parallel
        let prev_events = self.get_pdu_leaves(&room_id)?;

        // Our depth is the maximum depth of prev_events + 1
        let depth = prev_events
            .iter()
            .filter_map(|event_id| Some(self.get_pdu_json(event_id).ok()??.get("depth")?.as_u64()?))
            .max()
            .unwrap_or(0_u64)
            + 1;

        let mut unsigned = unsigned.unwrap_or_default();
        // TODO: Optimize this to not load the whole room state?
        if let Some(state_key) = &state_key {
            if let Some(prev_pdu) = self
                .room_state(&room_id)?
                .get(&(event_type.clone(), state_key.clone()))
            {
                unsigned.insert("prev_content".to_owned(), prev_pdu.content.clone());
            }
        }

        let mut pdu = PduEvent {
            event_id: EventId::try_from("$thiswillbefilledinlater").expect("we know this is valid"),
            room_id: room_id.clone(),
            sender: sender.clone(),
            origin: globals.hostname().to_owned(),
            origin_server_ts: utils::millis_since_unix_epoch()
                .try_into()
                .expect("this only fails many years in the future"),
            kind: event_type,
            content,
            state_key,
            prev_events,
            depth: depth
                .try_into()
                .expect("depth can overflow and should be deprecated..."),
            auth_events: Vec::new(),
            redacts: None,
            unsigned,
            hashes: ruma_federation_api::EventHash {
                sha256: "aaa".to_owned(),
            },
            signatures: HashMap::new(),
        };

        // Generate event id
        pdu.event_id = EventId::try_from(&*format!(
            "${}",
            ruma_signatures::reference_hash(&serde_json::to_value(&pdu)?)
                .expect("ruma can calculate reference hashes")
        ))
        .expect("ruma's reference hashes are correct");

        let mut pdu_json = serde_json::to_value(&pdu)?;
        ruma_signatures::hash_and_sign_event(globals.hostname(), globals.keypair(), &mut pdu_json)
            .expect("our new event can be hashed and signed");

        self.replace_pdu_leaves(&room_id, &pdu.event_id)?;

        // Increment the last index and use that
        // This is also the next_batch/since value
        let index = globals.next_count()?;

        let mut pdu_id = room_id.to_string().as_bytes().to_vec();
        pdu_id.push(0xff);
        pdu_id.extend_from_slice(&index.to_be_bytes());

        self.pduid_pdu.insert(&pdu_id, &*pdu_json.to_string())?;

        self.eventid_pduid
            .insert(pdu.event_id.to_string(), pdu_id)?;

        if let Some(state_key) = pdu.state_key {
            let mut key = room_id.to_string().as_bytes().to_vec();
            key.push(0xff);
            key.extend_from_slice(pdu.kind.to_string().as_bytes());
            key.push(0xff);
            key.extend_from_slice(state_key.as_bytes());
            self.roomstateid_pdu.insert(key, &*pdu_json.to_string())?;
        }

        self.edus.room_read_set(&room_id, &sender, index)?;

        Ok(pdu.event_id)
    }

    /// Returns an iterator over all PDUs in a room.
    pub fn all_pdus(&self, room_id: &RoomId) -> Result<impl Iterator<Item = Result<PduEvent>>> {
        self.pdus_since(room_id, 0)
    }

    /// Returns an iterator over all events in a room that happened after the event with id `since`.
    pub fn pdus_since(
        &self,
        room_id: &RoomId,
        since: u64,
    ) -> Result<impl Iterator<Item = Result<PduEvent>>> {
        // Create the first part of the full pdu id
        let mut pdu_id = room_id.to_string().as_bytes().to_vec();
        pdu_id.push(0xff);
        pdu_id.extend_from_slice(&(since).to_be_bytes());

        self.pdus_since_pduid(room_id, &pdu_id)
    }

    /// Returns an iterator over all events in a room that happened after the event with id `since`.
    pub fn pdus_since_pduid(
        &self,
        room_id: &RoomId,
        pdu_id: &[u8],
    ) -> Result<impl Iterator<Item = Result<PduEvent>>> {
        // Create the first part of the full pdu id
        let mut prefix = room_id.to_string().as_bytes().to_vec();
        prefix.push(0xff);

        Ok(self
            .pduid_pdu
            .range(pdu_id..)
            // Skip the first pdu if it's exactly at since, because we sent that last time
            .skip(if self.pduid_pdu.get(pdu_id)?.is_some() {
                1
            } else {
                0
            })
            .filter_map(|r| r.ok())
            .take_while(move |(k, _)| k.starts_with(&prefix))
            .map(|(_, v)| Ok(serde_json::from_slice(&v)?)))
    }

    /// Returns an iterator over all events in a room that happened before the event with id
    /// `until` in reverse-chronological order.
    pub fn pdus_until(
        &self,
        room_id: &RoomId,
        until: u64,
    ) -> impl Iterator<Item = Result<PduEvent>> {
        // Create the first part of the full pdu id
        let mut prefix = room_id.to_string().as_bytes().to_vec();
        prefix.push(0xff);

        let mut current = prefix.clone();
        current.extend_from_slice(&until.to_be_bytes());

        let current: &[u8] = &current;

        self.pduid_pdu
            .range(..current)
            .rev()
            .filter_map(|r| r.ok())
            .take_while(move |(k, _)| k.starts_with(&prefix))
            .map(|(_, v)| Ok(serde_json::from_slice(&v)?))
    }

    /// Makes a user join a room.
    pub fn join(
        &self,
        room_id: &RoomId,
        user_id: &UserId,
        displayname: Option<String>,
        globals: &super::globals::Globals,
    ) -> Result<()> {
        if !self.exists(room_id)? {
            return Err(Error::BadRequest("room does not exist"));
        }

        let mut userroom_id = user_id.to_string().as_bytes().to_vec();
        userroom_id.push(0xff);
        userroom_id.extend_from_slice(room_id.to_string().as_bytes());

        let mut roomuser_id = room_id.to_string().as_bytes().to_vec();
        roomuser_id.push(0xff);
        roomuser_id.extend_from_slice(user_id.to_string().as_bytes());

        self.userroomid_joined.insert(&userroom_id, &[])?;
        self.roomuserid_joined.insert(&roomuser_id, &[])?;
        self.userroomid_invited.remove(&userroom_id)?;
        self.roomuserid_invited.remove(&roomuser_id)?;
        self.userroomid_left.remove(&userroom_id)?;

        let mut content = json!({"membership": "join"});
        if let Some(displayname) = displayname {
            content
                .as_object_mut()
                .unwrap()
                .insert("displayname".to_owned(), displayname.into());
        }

        self.append_pdu(
            room_id.clone(),
            user_id.clone(),
            EventType::RoomMember,
            content,
            None,
            Some(user_id.to_string()),
            globals,
        )?;

        Ok(())
    }

    /// Makes a user leave a room.
    pub fn leave(
        &self,
        sender: &UserId,
        room_id: &RoomId,
        user_id: &UserId,
        globals: &super::globals::Globals,
    ) -> Result<()> {
        let mut userroom_id = user_id.to_string().as_bytes().to_vec();
        userroom_id.push(0xff);
        userroom_id.extend_from_slice(room_id.to_string().as_bytes());

        let mut roomuser_id = room_id.to_string().as_bytes().to_vec();
        roomuser_id.push(0xff);
        roomuser_id.extend_from_slice(user_id.to_string().as_bytes());

        self.userroomid_joined.remove(&userroom_id)?;
        self.roomuserid_joined.remove(&roomuser_id)?;
        self.userroomid_invited.remove(&userroom_id)?;
        self.roomuserid_invited.remove(&userroom_id)?;
        self.userroomid_left.insert(&userroom_id, &[])?;

        self.append_pdu(
            room_id.clone(),
            sender.clone(),
            EventType::RoomMember,
            json!({"membership": "leave"}),
            None,
            Some(user_id.to_string()),
            globals,
        )?;

        Ok(())
    }

    /// Makes a user forget a room.
    pub fn forget(&self, room_id: &RoomId, user_id: &UserId) -> Result<()> {
        let mut userroom_id = user_id.to_string().as_bytes().to_vec();
        userroom_id.push(0xff);
        userroom_id.extend_from_slice(room_id.to_string().as_bytes());

        self.userroomid_left.remove(userroom_id)?;

        Ok(())
    }

    /// Makes a user invite another user into room.
    pub fn invite(
        &self,
        sender: &UserId,
        room_id: &RoomId,
        user_id: &UserId,
        globals: &super::globals::Globals,
    ) -> Result<()> {
        let mut userroom_id = user_id.to_string().as_bytes().to_vec();
        userroom_id.push(0xff);
        userroom_id.extend_from_slice(room_id.to_string().as_bytes());

        let mut roomuser_id = room_id.to_string().as_bytes().to_vec();
        roomuser_id.push(0xff);
        roomuser_id.extend_from_slice(user_id.to_string().as_bytes());

        self.userroomid_invited.insert(userroom_id, &[])?;
        self.roomuserid_invited.insert(roomuser_id, &[])?;

        self.append_pdu(
            room_id.clone(),
            sender.clone(),
            EventType::RoomMember,
            json!({"membership": "invite"}),
            None,
            Some(user_id.to_string()),
            globals,
        )?;

        Ok(())
    }

    /// Returns an iterator over all rooms a user joined.
    pub fn room_members(&self, room_id: &RoomId) -> impl Iterator<Item = Result<UserId>> {
        self.roomuserid_joined
            .scan_prefix(room_id.to_string())
            .values()
            .map(|key| {
                Ok(UserId::try_from(&*utils::string_from_bytes(
                    &key?
                        .rsplit(|&b| b == 0xff)
                        .next()
                        .ok_or(Error::BadDatabase("userroomid is invalid"))?,
                )?)?)
            })
    }

    /// Returns an iterator over all rooms a user joined.
    pub fn room_members_invited(&self, room_id: &RoomId) -> impl Iterator<Item = Result<UserId>> {
        self.roomuserid_invited
            .scan_prefix(room_id.to_string())
            .keys()
            .map(|key| {
                Ok(UserId::try_from(&*utils::string_from_bytes(
                    &key?
                        .rsplit(|&b| b == 0xff)
                        .next()
                        .ok_or(Error::BadDatabase("userroomid is invalid"))?,
                )?)?)
            })
    }

    /// Returns an iterator over all rooms a user joined.
    pub fn rooms_joined(&self, user_id: &UserId) -> impl Iterator<Item = Result<RoomId>> {
        self.userroomid_joined
            .scan_prefix(user_id.to_string())
            .keys()
            .map(|key| {
                Ok(RoomId::try_from(&*utils::string_from_bytes(
                    &key?
                        .rsplit(|&b| b == 0xff)
                        .next()
                        .ok_or(Error::BadDatabase("userroomid is invalid"))?,
                )?)?)
            })
    }

    /// Returns an iterator over all rooms a user was invited to.
    pub fn rooms_invited(&self, user_id: &UserId) -> impl Iterator<Item = Result<RoomId>> {
        self.userroomid_invited
            .scan_prefix(&user_id.to_string())
            .keys()
            .map(|key| {
                Ok(RoomId::try_from(&*utils::string_from_bytes(
                    &key?
                        .rsplit(|&b| b == 0xff)
                        .next()
                        .ok_or(Error::BadDatabase("userroomid is invalid"))?,
                )?)?)
            })
    }

    /// Returns an iterator over all rooms a user left.
    pub fn rooms_left(&self, user_id: &UserId) -> impl Iterator<Item = Result<RoomId>> {
        self.userroomid_left
            .scan_prefix(&user_id.to_string())
            .keys()
            .map(|key| {
                Ok(RoomId::try_from(&*utils::string_from_bytes(
                    &key?
                        .rsplit(|&b| b == 0xff)
                        .next()
                        .ok_or(Error::BadDatabase("userroomid is invalid"))?,
                )?)?)
            })
    }
}
