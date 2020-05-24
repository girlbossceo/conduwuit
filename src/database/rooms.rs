mod edus;

pub use edus::RoomEdus;

use crate::{utils, Error, PduEvent, Result};
use log::error;
use ruma_events::{
    room::{
        join_rules, member,
        power_levels::{self, PowerLevelsEventContent},
    },
    EventJson, EventType,
};
use ruma_identifiers::{EventId, RoomId, UserId};
use sled::IVec;
use std::{
    collections::{BTreeMap, HashMap},
    convert::{TryFrom, TryInto},
    mem,
};

pub struct Rooms {
    pub edus: edus::RoomEdus,
    pub(super) pduid_pdu: sled::Tree, // PduId = RoomId + Count
    pub(super) eventid_pduid: sled::Tree,
    pub(super) roomid_pduleaves: sled::Tree,
    pub(super) roomstateid_pdu: sled::Tree, // RoomStateId = Room + StateType + StateKey

    pub(super) alias_roomid: sled::Tree,

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
                Ok(Some(serde_json::from_slice(
                    &self.pduid_pdu.get(pdu_id)?.ok_or(Error::BadDatabase(
                        "eventid_pduid points to nonexistent pdu",
                    ))?,
                )?))
            })
    }

    /// Returns the pdu's id.
    pub fn get_pdu_id(&self, event_id: &EventId) -> Result<Option<IVec>> {
        self.eventid_pduid
            .get(event_id.to_string().as_bytes())?
            .map_or(Ok(None), |pdu_id| Ok(Some(pdu_id)))
    }

    /// Returns the pdu.
    pub fn get_pdu(&self, event_id: &EventId) -> Result<Option<PduEvent>> {
        self.eventid_pduid
            .get(event_id.to_string().as_bytes())?
            .map_or(Ok(None), |pdu_id| {
                Ok(Some(serde_json::from_slice(
                    &self.pduid_pdu.get(pdu_id)?.ok_or(Error::BadDatabase(
                        "eventid_pduid points to nonexistent pdu",
                    ))?,
                )?))
            })
    }
    /// Returns the pdu.
    pub fn get_pdu_from_id(&self, pdu_id: &IVec) -> Result<Option<PduEvent>> {
        self.pduid_pdu
            .get(pdu_id)?
            .map_or(Ok(None), |pdu| Ok(Some(serde_json::from_slice(&pdu)?)))
    }

    /// Returns the pdu.
    pub fn replace_pdu(&self, pdu_id: &IVec, pdu: &PduEvent) -> Result<()> {
        if self.pduid_pdu.get(&pdu_id)?.is_some() {
            self.pduid_pdu
                .insert(&pdu_id, &*serde_json::to_string(pdu)?)?;
            Ok(())
        } else {
            Err(Error::BadRequest("pdu does not exist"))
        }
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
        redacts: Option<EventId>,
        globals: &super::globals::Globals,
    ) -> Result<EventId> {
        // TODO: Make sure this isn't called twice in parallel

        let prev_events = self.get_pdu_leaves(&room_id)?;

        // Is the event authorized?
        if let Some(state_key) = &state_key {
            let power_levels = self
                .room_state(&room_id)?
                .get(&(EventType::RoomPowerLevels, "".to_owned()))
                .map_or_else(
                    || {
                        Ok::<_, Error>(power_levels::PowerLevelsEventContent {
                            ban: 50.into(),
                            events: BTreeMap::new(),
                            events_default: 0.into(),
                            invite: 50.into(),
                            kick: 50.into(),
                            redact: 50.into(),
                            state_default: 0.into(),
                            users: BTreeMap::new(),
                            users_default: 0.into(),
                            notifications:
                                ruma_events::room::power_levels::NotificationPowerLevels {
                                    room: 50.into(),
                                },
                        })
                    },
                    |power_levels| {
                        Ok(
                            serde_json::from_value::<EventJson<PowerLevelsEventContent>>(
                                power_levels.content.clone(),
                            )?
                            .deserialize()?,
                        )
                    },
                )?;
            {
                let sender_membership = self
                    .room_state(&room_id)?
                    .get(&(EventType::RoomMember, sender.to_string()))
                    .map_or(Ok::<_, Error>(member::MembershipState::Leave), |pdu| {
                        Ok(
                            serde_json::from_value::<EventJson<member::MemberEventContent>>(
                                pdu.content.clone(),
                            )?
                            .deserialize()?
                            .membership,
                        )
                    })?;

                let sender_power = power_levels.users.get(&sender).map_or_else(
                    || {
                        if sender_membership != member::MembershipState::Join {
                            None
                        } else {
                            Some(&power_levels.users_default)
                        }
                    },
                    // If it's okay, wrap with Some(_)
                    Some,
                );

                if !match event_type {
                    EventType::RoomMember => {
                        let target_user_id = UserId::try_from(&**state_key)?;

                        let current_membership = self
                            .room_state(&room_id)?
                            .get(&(EventType::RoomMember, target_user_id.to_string()))
                            .map_or(Ok::<_, Error>(member::MembershipState::Leave), |pdu| {
                                Ok(serde_json::from_value::<
                                        EventJson<member::MemberEventContent>,
                                    >(pdu.content.clone())?
                                    .deserialize()?
                                    .membership)
                            })?;

                        let target_membership = serde_json::from_value::<
                            EventJson<member::MemberEventContent>,
                        >(content.clone())?
                        .deserialize()?
                        .membership;

                        let target_power = power_levels.users.get(&target_user_id).map_or_else(
                            || {
                                if target_membership != member::MembershipState::Join {
                                    None
                                } else {
                                    Some(&power_levels.users_default)
                                }
                            },
                            // If it's okay, wrap with Some(_)
                            Some,
                        );

                        let join_rules = self
                            .room_state(&room_id)?
                            .get(&(EventType::RoomJoinRules, "".to_owned()))
                            .map_or(join_rules::JoinRule::Public, |pdu| {
                                serde_json::from_value::<
                                    EventJson<join_rules::JoinRulesEventContent>,
                                >(pdu.content.clone())
                                .unwrap()
                                .deserialize()
                                .unwrap()
                                .join_rule
                            });

                        if target_membership == member::MembershipState::Join {
                            let mut prev_events = prev_events.iter();
                            let prev_event = self
                                .get_pdu(prev_events.next().ok_or(Error::BadRequest(
                                    "membership can't be the first event",
                                ))?)?
                                .ok_or(Error::BadDatabase("pdu leave points to valid event"))?;
                            if prev_event.kind == EventType::RoomCreate
                                && prev_event.prev_events.is_empty()
                            {
                                true
                            } else if sender != target_user_id {
                                false
                            } else if let member::MembershipState::Ban = current_membership {
                                false
                            } else if join_rules == join_rules::JoinRule::Invite
                                && (current_membership == member::MembershipState::Join
                                    || current_membership == member::MembershipState::Invite)
                            {
                                true
                            } else if join_rules == join_rules::JoinRule::Public {
                                true
                            } else {
                                false
                            }
                        } else if target_membership == member::MembershipState::Invite {
                            if let Some(third_party_invite_json) = content.get("third_party_invite")
                            {
                                if current_membership == member::MembershipState::Ban {
                                    false
                                } else {
                                    let _third_party_invite =
                                        serde_json::from_value::<member::ThirdPartyInvite>(
                                            third_party_invite_json.clone(),
                                        )?;
                                    todo!("handle third party invites");
                                }
                            } else if sender_membership != member::MembershipState::Join {
                                false
                            } else if current_membership == member::MembershipState::Join
                                || current_membership == member::MembershipState::Ban
                            {
                                false
                            } else if sender_power
                                .filter(|&p| p >= &power_levels.invite)
                                .is_some()
                            {
                                true
                            } else {
                                false
                            }
                        } else if target_membership == member::MembershipState::Leave {
                            if sender == target_user_id {
                                current_membership == member::MembershipState::Join
                                    || current_membership == member::MembershipState::Invite
                            } else if sender_membership != member::MembershipState::Join {
                                false
                            } else if current_membership == member::MembershipState::Ban
                                && sender_power.filter(|&p| p < &power_levels.ban).is_some()
                            {
                                false
                            } else if sender_power.filter(|&p| p >= &power_levels.kick).is_some()
                                && target_power < sender_power
                            {
                                true
                            } else {
                                false
                            }
                        } else if target_membership == member::MembershipState::Ban {
                            if sender_membership != member::MembershipState::Join {
                                false
                            } else if sender_power.filter(|&p| p >= &power_levels.ban).is_some()
                                && target_power < sender_power
                            {
                                true
                            } else {
                                false
                            }
                        } else {
                            false
                        }
                    }
                    EventType::RoomCreate => prev_events.is_empty(),
                    _ if sender_membership == member::MembershipState::Join => {
                        // TODO
                        sender_power.unwrap_or(&power_levels.users_default)
                            >= &power_levels.state_default
                    }

                    _ => false,
                } {
                    error!("Unauthorized");
                    // Not authorized
                    return Err(Error::BadRequest("event not authorized"));
                }
                if event_type == EventType::RoomMember {
                    // TODO: Don't get this twice
                    let target_user_id = UserId::try_from(&**state_key)?;
                    self.update_membership(
                        &room_id,
                        &target_user_id,
                        &serde_json::from_value::<EventJson<member::MemberEventContent>>(
                            content.clone(),
                        )?
                        .deserialize()?
                        .membership,
                    )?;
                }
            }
        } else if !self.is_joined(&sender, &room_id)? {
            return Err(Error::BadRequest("event not authorized"));
        }

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
                .get(&(event_type.clone(), state_key.to_owned()))
            {
                unsigned.insert("prev_content".to_owned(), prev_pdu.content.clone());
            }
        }

        let mut pdu = PduEvent {
            event_id: EventId::try_from("$thiswillbefilledinlater").expect("we know this is valid"),
            room_id: room_id.clone(),
            sender: sender.clone(),
            origin: globals.server_name().to_owned(),
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
            redacts,
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
        ruma_signatures::hash_and_sign_event(
            globals.server_name(),
            globals.keypair(),
            &mut pdu_json,
        )
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

    /// Replace a PDU with the redacted form.
    pub fn redact_pdu(&self, event_id: &EventId) -> Result<()> {
        if let Some(pdu_id) = self.get_pdu_id(event_id)? {
            let mut pdu = self
                .get_pdu_from_id(&pdu_id)?
                .ok_or(Error::BadDatabase("pduid points to invalid pdu"))?;
            pdu.redact();
            self.replace_pdu(&pdu_id, &pdu)?;
            Ok(())
        } else {
            Err(Error::BadRequest("eventid does not exist"))
        }
    }

    /// Update current membership data.
    fn update_membership(
        &self,
        room_id: &RoomId,
        user_id: &UserId,
        membership: &member::MembershipState,
    ) -> Result<()> {
        let mut userroom_id = user_id.to_string().as_bytes().to_vec();
        userroom_id.push(0xff);
        userroom_id.extend_from_slice(room_id.to_string().as_bytes());

        let mut roomuser_id = room_id.to_string().as_bytes().to_vec();
        roomuser_id.push(0xff);
        roomuser_id.extend_from_slice(user_id.to_string().as_bytes());

        match &membership {
            member::MembershipState::Join => {
                self.userroomid_joined.insert(&userroom_id, &[])?;
                self.roomuserid_joined.insert(&roomuser_id, &[])?;
                self.userroomid_invited.remove(&userroom_id)?;
                self.roomuserid_invited.remove(&roomuser_id)?;
                self.userroomid_left.remove(&userroom_id)?;
            }
            member::MembershipState::Invite => {
                self.userroomid_invited.insert(&userroom_id, &[])?;
                self.roomuserid_invited.insert(&roomuser_id, &[])?;
                self.userroomid_joined.remove(&userroom_id)?;
                self.roomuserid_joined.remove(&roomuser_id)?;
                self.userroomid_left.remove(&userroom_id)?;
            }
            member::MembershipState::Leave | member::MembershipState::Ban => {
                self.userroomid_left.insert(&userroom_id, &[])?;
                self.userroomid_joined.remove(&userroom_id)?;
                self.roomuserid_joined.remove(&roomuser_id)?;
                self.userroomid_invited.remove(&userroom_id)?;
                self.roomuserid_invited.remove(&roomuser_id)?;
            }
            _ => {}
        }

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

    pub fn id_from_alias(&self, alias: &str) -> Result<Option<RoomId>> {
        if !alias.starts_with('#') {
            return Err(Error::BadRequest("room alias does not start with #"));
        }

        self.alias_roomid.get(alias)?.map_or(Ok(None), |bytes| {
            Ok(Some(RoomId::try_from(utils::string_from_bytes(&bytes)?)?))
        })
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

    pub fn is_joined(&self, user_id: &UserId, room_id: &RoomId) -> Result<bool> {
        let mut userroom_id = user_id.to_string().as_bytes().to_vec();
        userroom_id.push(0xff);
        userroom_id.extend_from_slice(room_id.to_string().as_bytes());

        Ok(self.userroomid_joined.get(userroom_id)?.is_some())
    }

    pub fn is_invited(&self, user_id: &UserId, room_id: &RoomId) -> Result<bool> {
        let mut userroom_id = user_id.to_string().as_bytes().to_vec();
        userroom_id.push(0xff);
        userroom_id.extend_from_slice(room_id.to_string().as_bytes());

        Ok(self.userroomid_invited.get(userroom_id)?.is_some())
    }

    pub fn is_left(&self, user_id: &UserId, room_id: &RoomId) -> Result<bool> {
        let mut userroom_id = user_id.to_string().as_bytes().to_vec();
        userroom_id.push(0xff);
        userroom_id.extend_from_slice(room_id.to_string().as_bytes());

        Ok(self.userroomid_left.get(userroom_id)?.is_some())
    }
}
