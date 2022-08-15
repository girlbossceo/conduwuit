pub trait Data {
    fn get_room_shortstatehash(room_id: &RoomId);
}

    /// Returns the last state hash key added to the db for the given room.
    #[tracing::instrument(skip(self))]
    pub fn current_shortstatehash(&self, room_id: &RoomId) -> Result<Option<u64>> {
        self.roomid_shortstatehash
            .get(room_id.as_bytes())?
            .map_or(Ok(None), |bytes| {
                Ok(Some(utils::u64_from_bytes(&bytes).map_err(|_| {
                    Error::bad_database("Invalid shortstatehash in roomid_shortstatehash")
                })?))
            })
    }

pub struct Service<D: Data> {
    db: D,
}

impl Service {
    /// Set the room to the given statehash and update caches.
    #[tracing::instrument(skip(self, new_state_ids_compressed, db))]
    pub fn force_state(
        &self,
        room_id: &RoomId,
        shortstatehash: u64,
        statediffnew :HashSet<CompressedStateEvent>,
        statediffremoved :HashSet<CompressedStateEvent>,
        db: &Database,
    ) -> Result<()> {

        for event_id in statediffnew.into_iter().filter_map(|new| {
            self.parse_compressed_state_event(new)
                .ok()
                .map(|(_, id)| id)
        }) {
            let pdu = match self.get_pdu_json(&event_id)? {
                Some(pdu) => pdu,
                None => continue,
            };

            if pdu.get("type").and_then(|val| val.as_str()) != Some("m.room.member") {
                continue;
            }

            let pdu: PduEvent = match serde_json::from_str(
                &serde_json::to_string(&pdu).expect("CanonicalJsonObj can be serialized to JSON"),
            ) {
                Ok(pdu) => pdu,
                Err(_) => continue,
            };

            #[derive(Deserialize)]
            struct ExtractMembership {
                membership: MembershipState,
            }

            let membership = match serde_json::from_str::<ExtractMembership>(pdu.content.get()) {
                Ok(e) => e.membership,
                Err(_) => continue,
            };

            let state_key = match pdu.state_key {
                Some(k) => k,
                None => continue,
            };

            let user_id = match UserId::parse(state_key) {
                Ok(id) => id,
                Err(_) => continue,
            };

            self.update_membership(room_id, &user_id, membership, &pdu.sender, None, db, false)?;
        }

        self.update_joined_count(room_id, db)?;

        self.roomid_shortstatehash
            .insert(room_id.as_bytes(), &new_shortstatehash.to_be_bytes())?;

        Ok(())
    }

    /// Returns the leaf pdus of a room.
    #[tracing::instrument(skip(self))]
    pub fn get_pdu_leaves(&self, room_id: &RoomId) -> Result<HashSet<Arc<EventId>>> {
        let mut prefix = room_id.as_bytes().to_vec();
        prefix.push(0xff);

        self.roomid_pduleaves
            .scan_prefix(prefix)
            .map(|(_, bytes)| {
                EventId::parse_arc(utils::string_from_bytes(&bytes).map_err(|_| {
                    Error::bad_database("EventID in roomid_pduleaves is invalid unicode.")
                })?)
                .map_err(|_| Error::bad_database("EventId in roomid_pduleaves is invalid."))
            })
            .collect()
    }

    /// Replace the leaves of a room.
    ///
    /// The provided `event_ids` become the new leaves, this allows a room to have multiple
    /// `prev_events`.
    #[tracing::instrument(skip(self))]
    pub fn replace_pdu_leaves<'a>(
        &self,
        room_id: &RoomId,
        event_ids: impl IntoIterator<Item = &'a EventId> + Debug,
    ) -> Result<()> {
        let mut prefix = room_id.as_bytes().to_vec();
        prefix.push(0xff);

        for (key, _) in self.roomid_pduleaves.scan_prefix(prefix.clone()) {
            self.roomid_pduleaves.remove(&key)?;
        }

        for event_id in event_ids {
            let mut key = prefix.to_owned();
            key.extend_from_slice(event_id.as_bytes());
            self.roomid_pduleaves.insert(&key, event_id.as_bytes())?;
        }

        Ok(())
    }

    /// Generates a new StateHash and associates it with the incoming event.
    ///
    /// This adds all current state events (not including the incoming event)
    /// to `stateid_pduid` and adds the incoming event to `eventid_statehash`.
    #[tracing::instrument(skip(self, state_ids_compressed, globals))]
    pub fn set_event_state(
        &self,
        event_id: &EventId,
        room_id: &RoomId,
        state_ids_compressed: HashSet<CompressedStateEvent>,
        globals: &super::globals::Globals,
    ) -> Result<()> {
        let shorteventid = self.get_or_create_shorteventid(event_id, globals)?;

        let previous_shortstatehash = self.current_shortstatehash(room_id)?;

        let state_hash = self.calculate_hash(
            &state_ids_compressed
                .iter()
                .map(|s| &s[..])
                .collect::<Vec<_>>(),
        );

        let (shortstatehash, already_existed) =
            self.get_or_create_shortstatehash(&state_hash, globals)?;

        if !already_existed {
            let states_parents = previous_shortstatehash
                .map_or_else(|| Ok(Vec::new()), |p| self.load_shortstatehash_info(p))?;

            let (statediffnew, statediffremoved) =
                if let Some(parent_stateinfo) = states_parents.last() {
                    let statediffnew: HashSet<_> = state_ids_compressed
                        .difference(&parent_stateinfo.1)
                        .copied()
                        .collect();

                    let statediffremoved: HashSet<_> = parent_stateinfo
                        .1
                        .difference(&state_ids_compressed)
                        .copied()
                        .collect();

                    (statediffnew, statediffremoved)
                } else {
                    (state_ids_compressed, HashSet::new())
                };
            self.save_state_from_diff(
                shortstatehash,
                statediffnew,
                statediffremoved,
                1_000_000, // high number because no state will be based on this one
                states_parents,
            )?;
        }

        self.shorteventid_shortstatehash
            .insert(&shorteventid.to_be_bytes(), &shortstatehash.to_be_bytes())?;

        Ok(())
    }

    /// Generates a new StateHash and associates it with the incoming event.
    ///
    /// This adds all current state events (not including the incoming event)
    /// to `stateid_pduid` and adds the incoming event to `eventid_statehash`.
    #[tracing::instrument(skip(self, new_pdu, globals))]
    pub fn append_to_state(
        &self,
        new_pdu: &PduEvent,
        globals: &super::globals::Globals,
    ) -> Result<u64> {
        let shorteventid = self.get_or_create_shorteventid(&new_pdu.event_id, globals)?;

        let previous_shortstatehash = self.current_shortstatehash(&new_pdu.room_id)?;

        if let Some(p) = previous_shortstatehash {
            self.shorteventid_shortstatehash
                .insert(&shorteventid.to_be_bytes(), &p.to_be_bytes())?;
        }

        if let Some(state_key) = &new_pdu.state_key {
            let states_parents = previous_shortstatehash
                .map_or_else(|| Ok(Vec::new()), |p| self.load_shortstatehash_info(p))?;

            let shortstatekey = self.get_or_create_shortstatekey(
                &new_pdu.kind.to_string().into(),
                state_key,
                globals,
            )?;

            let new = self.compress_state_event(shortstatekey, &new_pdu.event_id, globals)?;

            let replaces = states_parents
                .last()
                .map(|info| {
                    info.1
                        .iter()
                        .find(|bytes| bytes.starts_with(&shortstatekey.to_be_bytes()))
                })
                .unwrap_or_default();

            if Some(&new) == replaces {
                return Ok(previous_shortstatehash.expect("must exist"));
            }

            // TODO: statehash with deterministic inputs
            let shortstatehash = globals.next_count()?;

            let mut statediffnew = HashSet::new();
            statediffnew.insert(new);

            let mut statediffremoved = HashSet::new();
            if let Some(replaces) = replaces {
                statediffremoved.insert(*replaces);
            }

            self.save_state_from_diff(
                shortstatehash,
                statediffnew,
                statediffremoved,
                2,
                states_parents,
            )?;

            Ok(shortstatehash)
        } else {
            Ok(previous_shortstatehash.expect("first event in room must be a state event"))
        }
    }

    #[tracing::instrument(skip(self, invite_event))]
    pub fn calculate_invite_state(
        &self,
        invite_event: &PduEvent,
    ) -> Result<Vec<Raw<AnyStrippedStateEvent>>> {
        let mut state = Vec::new();
        // Add recommended events
        if let Some(e) =
            self.room_state_get(&invite_event.room_id, &StateEventType::RoomCreate, "")?
        {
            state.push(e.to_stripped_state_event());
        }
        if let Some(e) =
            self.room_state_get(&invite_event.room_id, &StateEventType::RoomJoinRules, "")?
        {
            state.push(e.to_stripped_state_event());
        }
        if let Some(e) = self.room_state_get(
            &invite_event.room_id,
            &StateEventType::RoomCanonicalAlias,
            "",
        )? {
            state.push(e.to_stripped_state_event());
        }
        if let Some(e) =
            self.room_state_get(&invite_event.room_id, &StateEventType::RoomAvatar, "")?
        {
            state.push(e.to_stripped_state_event());
        }
        if let Some(e) =
            self.room_state_get(&invite_event.room_id, &StateEventType::RoomName, "")?
        {
            state.push(e.to_stripped_state_event());
        }
        if let Some(e) = self.room_state_get(
            &invite_event.room_id,
            &StateEventType::RoomMember,
            invite_event.sender.as_str(),
        )? {
            state.push(e.to_stripped_state_event());
        }

        state.push(invite_event.to_stripped_state_event());
        Ok(state)
    }

    #[tracing::instrument(skip(self))]
    pub fn set_room_state(&self, room_id: &RoomId, shortstatehash: u64) -> Result<()> {
        self.roomid_shortstatehash
            .insert(room_id.as_bytes(), &shortstatehash.to_be_bytes())?;

        Ok(())
    }
}

    #[tracing::instrument(skip(self, globals))]
    pub fn set_alias(
        &self,
        alias: &RoomAliasId,
        room_id: Option<&RoomId>,
        globals: &super::globals::Globals,
    ) -> Result<()> {
        if let Some(room_id) = room_id {
            // New alias
            self.alias_roomid
                .insert(alias.alias().as_bytes(), room_id.as_bytes())?;
            let mut aliasid = room_id.as_bytes().to_vec();
            aliasid.push(0xff);
            aliasid.extend_from_slice(&globals.next_count()?.to_be_bytes());
            self.aliasid_alias.insert(&aliasid, &*alias.as_bytes())?;
        } else {
            // room_id=None means remove alias
            if let Some(room_id) = self.alias_roomid.get(alias.alias().as_bytes())? {
                let mut prefix = room_id.to_vec();
                prefix.push(0xff);

                for (key, _) in self.aliasid_alias.scan_prefix(prefix) {
                    self.aliasid_alias.remove(&key)?;
                }
                self.alias_roomid.remove(alias.alias().as_bytes())?;
            } else {
                return Err(Error::BadRequest(
                    ErrorKind::NotFound,
                    "Alias does not exist.",
                ));
            }
        }

        Ok(())
    }

    #[tracing::instrument(skip(self))]
    pub fn id_from_alias(&self, alias: &RoomAliasId) -> Result<Option<Box<RoomId>>> {
        self.alias_roomid
            .get(alias.alias().as_bytes())?
            .map(|bytes| {
                RoomId::parse(utils::string_from_bytes(&bytes).map_err(|_| {
                    Error::bad_database("Room ID in alias_roomid is invalid unicode.")
                })?)
                .map_err(|_| Error::bad_database("Room ID in alias_roomid is invalid."))
            })
            .transpose()
    }

    #[tracing::instrument(skip(self))]
    pub fn room_aliases<'a>(
        &'a self,
        room_id: &RoomId,
    ) -> impl Iterator<Item = Result<Box<RoomAliasId>>> + 'a {
        let mut prefix = room_id.as_bytes().to_vec();
        prefix.push(0xff);

        self.aliasid_alias.scan_prefix(prefix).map(|(_, bytes)| {
            utils::string_from_bytes(&bytes)
                .map_err(|_| Error::bad_database("Invalid alias bytes in aliasid_alias."))?
                .try_into()
                .map_err(|_| Error::bad_database("Invalid alias in aliasid_alias."))
        })
    }


    #[tracing::instrument(skip(self))]
    pub fn set_public(&self, room_id: &RoomId, public: bool) -> Result<()> {
        if public {
            self.publicroomids.insert(room_id.as_bytes(), &[])?;
        } else {
            self.publicroomids.remove(room_id.as_bytes())?;
        }

        Ok(())
    }

    #[tracing::instrument(skip(self))]
    pub fn is_public_room(&self, room_id: &RoomId) -> Result<bool> {
        Ok(self.publicroomids.get(room_id.as_bytes())?.is_some())
    }

    #[tracing::instrument(skip(self))]
    pub fn public_rooms(&self) -> impl Iterator<Item = Result<Box<RoomId>>> + '_ {
        self.publicroomids.iter().map(|(bytes, _)| {
            RoomId::parse(
                utils::string_from_bytes(&bytes).map_err(|_| {
                    Error::bad_database("Room ID in publicroomids is invalid unicode.")
                })?,
            )
            .map_err(|_| Error::bad_database("Room ID in publicroomids is invalid."))
        })
    }

use crate::{database::abstraction::Tree, utils, Error, Result};
use ruma::{
    events::{
        presence::{PresenceEvent, PresenceEventContent},
        receipt::ReceiptEvent,
        SyncEphemeralRoomEvent,
    },
    presence::PresenceState,
    serde::Raw,
    signatures::CanonicalJsonObject,
    RoomId, UInt, UserId,
};
use std::{
    collections::{HashMap, HashSet},
    mem,
    sync::Arc,
};

pub struct RoomEdus {
    pub(in super::super) readreceiptid_readreceipt: Arc<dyn Tree>, // ReadReceiptId = RoomId + Count + UserId
    pub(in super::super) roomuserid_privateread: Arc<dyn Tree>, // RoomUserId = Room + User, PrivateRead = Count
    pub(in super::super) roomuserid_lastprivatereadupdate: Arc<dyn Tree>, // LastPrivateReadUpdate = Count
    pub(in super::super) typingid_userid: Arc<dyn Tree>, // TypingId = RoomId + TimeoutTime + Count
    pub(in super::super) roomid_lasttypingupdate: Arc<dyn Tree>, // LastRoomTypingUpdate = Count
    pub(in super::super) presenceid_presence: Arc<dyn Tree>, // PresenceId = RoomId + Count + UserId
    pub(in super::super) userid_lastpresenceupdate: Arc<dyn Tree>, // LastPresenceUpdate = Count
}

impl RoomEdus {
    /// Adds an event which will be saved until a new event replaces it (e.g. read receipt).
    pub fn readreceipt_update(
        &self,
        user_id: &UserId,
        room_id: &RoomId,
        event: ReceiptEvent,
        globals: &super::super::globals::Globals,
    ) -> Result<()> {
        let mut prefix = room_id.as_bytes().to_vec();
        prefix.push(0xff);

        let mut last_possible_key = prefix.clone();
        last_possible_key.extend_from_slice(&u64::MAX.to_be_bytes());

        // Remove old entry
        if let Some((old, _)) = self
            .readreceiptid_readreceipt
            .iter_from(&last_possible_key, true)
            .take_while(|(key, _)| key.starts_with(&prefix))
            .find(|(key, _)| {
                key.rsplit(|&b| b == 0xff)
                    .next()
                    .expect("rsplit always returns an element")
                    == user_id.as_bytes()
            })
        {
            // This is the old room_latest
            self.readreceiptid_readreceipt.remove(&old)?;
        }

        let mut room_latest_id = prefix;
        room_latest_id.extend_from_slice(&globals.next_count()?.to_be_bytes());
        room_latest_id.push(0xff);
        room_latest_id.extend_from_slice(user_id.as_bytes());

        self.readreceiptid_readreceipt.insert(
            &room_latest_id,
            &serde_json::to_vec(&event).expect("EduEvent::to_string always works"),
        )?;

        Ok(())
    }

    /// Returns an iterator over the most recent read_receipts in a room that happened after the event with id `since`.
    #[tracing::instrument(skip(self))]
    pub fn readreceipts_since<'a>(
        &'a self,
        room_id: &RoomId,
        since: u64,
    ) -> impl Iterator<
        Item = Result<(
            Box<UserId>,
            u64,
            Raw<ruma::events::AnySyncEphemeralRoomEvent>,
        )>,
    > + 'a {
        let mut prefix = room_id.as_bytes().to_vec();
        prefix.push(0xff);
        let prefix2 = prefix.clone();

        let mut first_possible_edu = prefix.clone();
        first_possible_edu.extend_from_slice(&(since + 1).to_be_bytes()); // +1 so we don't send the event at since

        self.readreceiptid_readreceipt
            .iter_from(&first_possible_edu, false)
            .take_while(move |(k, _)| k.starts_with(&prefix2))
            .map(move |(k, v)| {
                let count =
                    utils::u64_from_bytes(&k[prefix.len()..prefix.len() + mem::size_of::<u64>()])
                        .map_err(|_| Error::bad_database("Invalid readreceiptid count in db."))?;
                let user_id = UserId::parse(
                    utils::string_from_bytes(&k[prefix.len() + mem::size_of::<u64>() + 1..])
                        .map_err(|_| {
                            Error::bad_database("Invalid readreceiptid userid bytes in db.")
                        })?,
                )
                .map_err(|_| Error::bad_database("Invalid readreceiptid userid in db."))?;

                let mut json = serde_json::from_slice::<CanonicalJsonObject>(&v).map_err(|_| {
                    Error::bad_database("Read receipt in roomlatestid_roomlatest is invalid json.")
                })?;
                json.remove("room_id");

                Ok((
                    user_id,
                    count,
                    Raw::from_json(
                        serde_json::value::to_raw_value(&json).expect("json is valid raw value"),
                    ),
                ))
            })
    }

    /// Sets a private read marker at `count`.
    #[tracing::instrument(skip(self, globals))]
    pub fn private_read_set(
        &self,
        room_id: &RoomId,
        user_id: &UserId,
        count: u64,
        globals: &super::super::globals::Globals,
    ) -> Result<()> {
        let mut key = room_id.as_bytes().to_vec();
        key.push(0xff);
        key.extend_from_slice(user_id.as_bytes());

        self.roomuserid_privateread
            .insert(&key, &count.to_be_bytes())?;

        self.roomuserid_lastprivatereadupdate
            .insert(&key, &globals.next_count()?.to_be_bytes())?;

        Ok(())
    }

    /// Returns the private read marker.
    #[tracing::instrument(skip(self))]
    pub fn private_read_get(&self, room_id: &RoomId, user_id: &UserId) -> Result<Option<u64>> {
        let mut key = room_id.as_bytes().to_vec();
        key.push(0xff);
        key.extend_from_slice(user_id.as_bytes());

        self.roomuserid_privateread
            .get(&key)?
            .map_or(Ok(None), |v| {
                Ok(Some(utils::u64_from_bytes(&v).map_err(|_| {
                    Error::bad_database("Invalid private read marker bytes")
                })?))
            })
    }

    /// Returns the count of the last typing update in this room.
    pub fn last_privateread_update(&self, user_id: &UserId, room_id: &RoomId) -> Result<u64> {
        let mut key = room_id.as_bytes().to_vec();
        key.push(0xff);
        key.extend_from_slice(user_id.as_bytes());

        Ok(self
            .roomuserid_lastprivatereadupdate
            .get(&key)?
            .map(|bytes| {
                utils::u64_from_bytes(&bytes).map_err(|_| {
                    Error::bad_database("Count in roomuserid_lastprivatereadupdate is invalid.")
                })
            })
            .transpose()?
            .unwrap_or(0))
    }

    /// Sets a user as typing until the timeout timestamp is reached or roomtyping_remove is
    /// called.
    pub fn typing_add(
        &self,
        user_id: &UserId,
        room_id: &RoomId,
        timeout: u64,
        globals: &super::super::globals::Globals,
    ) -> Result<()> {
        let mut prefix = room_id.as_bytes().to_vec();
        prefix.push(0xff);

        let count = globals.next_count()?.to_be_bytes();

        let mut room_typing_id = prefix;
        room_typing_id.extend_from_slice(&timeout.to_be_bytes());
        room_typing_id.push(0xff);
        room_typing_id.extend_from_slice(&count);

        self.typingid_userid
            .insert(&room_typing_id, &*user_id.as_bytes())?;

        self.roomid_lasttypingupdate
            .insert(room_id.as_bytes(), &count)?;

        Ok(())
    }

    /// Removes a user from typing before the timeout is reached.
    pub fn typing_remove(
        &self,
        user_id: &UserId,
        room_id: &RoomId,
        globals: &super::super::globals::Globals,
    ) -> Result<()> {
        let mut prefix = room_id.as_bytes().to_vec();
        prefix.push(0xff);

        let user_id = user_id.to_string();

        let mut found_outdated = false;

        // Maybe there are multiple ones from calling roomtyping_add multiple times
        for outdated_edu in self
            .typingid_userid
            .scan_prefix(prefix)
            .filter(|(_, v)| &**v == user_id.as_bytes())
        {
            self.typingid_userid.remove(&outdated_edu.0)?;
            found_outdated = true;
        }

        if found_outdated {
            self.roomid_lasttypingupdate
                .insert(room_id.as_bytes(), &globals.next_count()?.to_be_bytes())?;
        }

        Ok(())
    }

    /// Makes sure that typing events with old timestamps get removed.
    fn typings_maintain(
        &self,
        room_id: &RoomId,
        globals: &super::super::globals::Globals,
    ) -> Result<()> {
        let mut prefix = room_id.as_bytes().to_vec();
        prefix.push(0xff);

        let current_timestamp = utils::millis_since_unix_epoch();

        let mut found_outdated = false;

        // Find all outdated edus before inserting a new one
        for outdated_edu in self
            .typingid_userid
            .scan_prefix(prefix)
            .map(|(key, _)| {
                Ok::<_, Error>((
                    key.clone(),
                    utils::u64_from_bytes(
                        &key.splitn(2, |&b| b == 0xff).nth(1).ok_or_else(|| {
                            Error::bad_database("RoomTyping has invalid timestamp or delimiters.")
                        })?[0..mem::size_of::<u64>()],
                    )
                    .map_err(|_| Error::bad_database("RoomTyping has invalid timestamp bytes."))?,
                ))
            })
            .filter_map(|r| r.ok())
            .take_while(|&(_, timestamp)| timestamp < current_timestamp)
        {
            // This is an outdated edu (time > timestamp)
            self.typingid_userid.remove(&outdated_edu.0)?;
            found_outdated = true;
        }

        if found_outdated {
            self.roomid_lasttypingupdate
                .insert(room_id.as_bytes(), &globals.next_count()?.to_be_bytes())?;
        }

        Ok(())
    }

    /// Returns the count of the last typing update in this room.
    #[tracing::instrument(skip(self, globals))]
    pub fn last_typing_update(
        &self,
        room_id: &RoomId,
        globals: &super::super::globals::Globals,
    ) -> Result<u64> {
        self.typings_maintain(room_id, globals)?;

        Ok(self
            .roomid_lasttypingupdate
            .get(room_id.as_bytes())?
            .map(|bytes| {
                utils::u64_from_bytes(&bytes).map_err(|_| {
                    Error::bad_database("Count in roomid_lastroomactiveupdate is invalid.")
                })
            })
            .transpose()?
            .unwrap_or(0))
    }

    pub fn typings_all(
        &self,
        room_id: &RoomId,
    ) -> Result<SyncEphemeralRoomEvent<ruma::events::typing::TypingEventContent>> {
        let mut prefix = room_id.as_bytes().to_vec();
        prefix.push(0xff);

        let mut user_ids = HashSet::new();

        for (_, user_id) in self.typingid_userid.scan_prefix(prefix) {
            let user_id = UserId::parse(utils::string_from_bytes(&user_id).map_err(|_| {
                Error::bad_database("User ID in typingid_userid is invalid unicode.")
            })?)
            .map_err(|_| Error::bad_database("User ID in typingid_userid is invalid."))?;

            user_ids.insert(user_id);
        }

        Ok(SyncEphemeralRoomEvent {
            content: ruma::events::typing::TypingEventContent {
                user_ids: user_ids.into_iter().collect(),
            },
        })
    }

    /// Adds a presence event which will be saved until a new event replaces it.
    ///
    /// Note: This method takes a RoomId because presence updates are always bound to rooms to
    /// make sure users outside these rooms can't see them.
    pub fn update_presence(
        &self,
        user_id: &UserId,
        room_id: &RoomId,
        presence: PresenceEvent,
        globals: &super::super::globals::Globals,
    ) -> Result<()> {
        // TODO: Remove old entry? Or maybe just wipe completely from time to time?

        let count = globals.next_count()?.to_be_bytes();

        let mut presence_id = room_id.as_bytes().to_vec();
        presence_id.push(0xff);
        presence_id.extend_from_slice(&count);
        presence_id.push(0xff);
        presence_id.extend_from_slice(presence.sender.as_bytes());

        self.presenceid_presence.insert(
            &presence_id,
            &serde_json::to_vec(&presence).expect("PresenceEvent can be serialized"),
        )?;

        self.userid_lastpresenceupdate.insert(
            user_id.as_bytes(),
            &utils::millis_since_unix_epoch().to_be_bytes(),
        )?;

        Ok(())
    }

    /// Resets the presence timeout, so the user will stay in their current presence state.
    #[tracing::instrument(skip(self))]
    pub fn ping_presence(&self, user_id: &UserId) -> Result<()> {
        self.userid_lastpresenceupdate.insert(
            user_id.as_bytes(),
            &utils::millis_since_unix_epoch().to_be_bytes(),
        )?;

        Ok(())
    }

    /// Returns the timestamp of the last presence update of this user in millis since the unix epoch.
    pub fn last_presence_update(&self, user_id: &UserId) -> Result<Option<u64>> {
        self.userid_lastpresenceupdate
            .get(user_id.as_bytes())?
            .map(|bytes| {
                utils::u64_from_bytes(&bytes).map_err(|_| {
                    Error::bad_database("Invalid timestamp in userid_lastpresenceupdate.")
                })
            })
            .transpose()
    }

    pub fn get_last_presence_event(
        &self,
        user_id: &UserId,
        room_id: &RoomId,
    ) -> Result<Option<PresenceEvent>> {
        let last_update = match self.last_presence_update(user_id)? {
            Some(last) => last,
            None => return Ok(None),
        };

        let mut presence_id = room_id.as_bytes().to_vec();
        presence_id.push(0xff);
        presence_id.extend_from_slice(&last_update.to_be_bytes());
        presence_id.push(0xff);
        presence_id.extend_from_slice(user_id.as_bytes());

        self.presenceid_presence
            .get(&presence_id)?
            .map(|value| {
                let mut presence: PresenceEvent = serde_json::from_slice(&value)
                    .map_err(|_| Error::bad_database("Invalid presence event in db."))?;
                let current_timestamp: UInt = utils::millis_since_unix_epoch()
                    .try_into()
                    .expect("time is valid");

                if presence.content.presence == PresenceState::Online {
                    // Don't set last_active_ago when the user is online
                    presence.content.last_active_ago = None;
                } else {
                    // Convert from timestamp to duration
                    presence.content.last_active_ago = presence
                        .content
                        .last_active_ago
                        .map(|timestamp| current_timestamp - timestamp);
                }

                Ok(presence)
            })
            .transpose()
    }

    /// Sets all users to offline who have been quiet for too long.
    fn _presence_maintain(
        &self,
        rooms: &super::Rooms,
        globals: &super::super::globals::Globals,
    ) -> Result<()> {
        let current_timestamp = utils::millis_since_unix_epoch();

        for (user_id_bytes, last_timestamp) in self
            .userid_lastpresenceupdate
            .iter()
            .filter_map(|(k, bytes)| {
                Some((
                    k,
                    utils::u64_from_bytes(&bytes)
                        .map_err(|_| {
                            Error::bad_database("Invalid timestamp in userid_lastpresenceupdate.")
                        })
                        .ok()?,
                ))
            })
            .take_while(|(_, timestamp)| current_timestamp.saturating_sub(*timestamp) > 5 * 60_000)
        // 5 Minutes
        {
            // Send new presence events to set the user offline
            let count = globals.next_count()?.to_be_bytes();
            let user_id: Box<_> = utils::string_from_bytes(&user_id_bytes)
                .map_err(|_| {
                    Error::bad_database("Invalid UserId bytes in userid_lastpresenceupdate.")
                })?
                .try_into()
                .map_err(|_| Error::bad_database("Invalid UserId in userid_lastpresenceupdate."))?;
            for room_id in rooms.rooms_joined(&user_id).filter_map(|r| r.ok()) {
                let mut presence_id = room_id.as_bytes().to_vec();
                presence_id.push(0xff);
                presence_id.extend_from_slice(&count);
                presence_id.push(0xff);
                presence_id.extend_from_slice(&user_id_bytes);

                self.presenceid_presence.insert(
                    &presence_id,
                    &serde_json::to_vec(&PresenceEvent {
                        content: PresenceEventContent {
                            avatar_url: None,
                            currently_active: None,
                            displayname: None,
                            last_active_ago: Some(
                                last_timestamp.try_into().expect("time is valid"),
                            ),
                            presence: PresenceState::Offline,
                            status_msg: None,
                        },
                        sender: user_id.to_owned(),
                    })
                    .expect("PresenceEvent can be serialized"),
                )?;
            }

            self.userid_lastpresenceupdate.insert(
                user_id.as_bytes(),
                &utils::millis_since_unix_epoch().to_be_bytes(),
            )?;
        }

        Ok(())
    }

    /// Returns an iterator over the most recent presence updates that happened after the event with id `since`.
    #[tracing::instrument(skip(self, since, _rooms, _globals))]
    pub fn presence_since(
        &self,
        room_id: &RoomId,
        since: u64,
        _rooms: &super::Rooms,
        _globals: &super::super::globals::Globals,
    ) -> Result<HashMap<Box<UserId>, PresenceEvent>> {
        //self.presence_maintain(rooms, globals)?;

        let mut prefix = room_id.as_bytes().to_vec();
        prefix.push(0xff);

        let mut first_possible_edu = prefix.clone();
        first_possible_edu.extend_from_slice(&(since + 1).to_be_bytes()); // +1 so we don't send the event at since
        let mut hashmap = HashMap::new();

        for (key, value) in self
            .presenceid_presence
            .iter_from(&*first_possible_edu, false)
            .take_while(|(key, _)| key.starts_with(&prefix))
        {
            let user_id = UserId::parse(
                utils::string_from_bytes(
                    key.rsplit(|&b| b == 0xff)
                        .next()
                        .expect("rsplit always returns an element"),
                )
                .map_err(|_| Error::bad_database("Invalid UserId bytes in presenceid_presence."))?,
            )
            .map_err(|_| Error::bad_database("Invalid UserId in presenceid_presence."))?;

            let mut presence: PresenceEvent = serde_json::from_slice(&value)
                .map_err(|_| Error::bad_database("Invalid presence event in db."))?;

            let current_timestamp: UInt = utils::millis_since_unix_epoch()
                .try_into()
                .expect("time is valid");

            if presence.content.presence == PresenceState::Online {
                // Don't set last_active_ago when the user is online
                presence.content.last_active_ago = None;
            } else {
                // Convert from timestamp to duration
                presence.content.last_active_ago = presence
                    .content
                    .last_active_ago
                    .map(|timestamp| current_timestamp - timestamp);
            }

            hashmap.insert(user_id, presence);
        }

        Ok(hashmap)
    }
}

    #[tracing::instrument(skip(self))]
    pub fn lazy_load_was_sent_before(
        &self,
        user_id: &UserId,
        device_id: &DeviceId,
        room_id: &RoomId,
        ll_user: &UserId,
    ) -> Result<bool> {
        let mut key = user_id.as_bytes().to_vec();
        key.push(0xff);
        key.extend_from_slice(device_id.as_bytes());
        key.push(0xff);
        key.extend_from_slice(room_id.as_bytes());
        key.push(0xff);
        key.extend_from_slice(ll_user.as_bytes());
        Ok(self.lazyloadedids.get(&key)?.is_some())
    }

    #[tracing::instrument(skip(self))]
    pub fn lazy_load_mark_sent(
        &self,
        user_id: &UserId,
        device_id: &DeviceId,
        room_id: &RoomId,
        lazy_load: HashSet<Box<UserId>>,
        count: u64,
    ) {
        self.lazy_load_waiting.lock().unwrap().insert(
            (
                user_id.to_owned(),
                device_id.to_owned(),
                room_id.to_owned(),
                count,
            ),
            lazy_load,
        );
    }

    #[tracing::instrument(skip(self))]
    pub fn lazy_load_confirm_delivery(
        &self,
        user_id: &UserId,
        device_id: &DeviceId,
        room_id: &RoomId,
        since: u64,
    ) -> Result<()> {
        if let Some(user_ids) = self.lazy_load_waiting.lock().unwrap().remove(&(
            user_id.to_owned(),
            device_id.to_owned(),
            room_id.to_owned(),
            since,
        )) {
            let mut prefix = user_id.as_bytes().to_vec();
            prefix.push(0xff);
            prefix.extend_from_slice(device_id.as_bytes());
            prefix.push(0xff);
            prefix.extend_from_slice(room_id.as_bytes());
            prefix.push(0xff);

            for ll_id in user_ids {
                let mut key = prefix.clone();
                key.extend_from_slice(ll_id.as_bytes());
                self.lazyloadedids.insert(&key, &[])?;
            }
        }

        Ok(())
    }

    #[tracing::instrument(skip(self))]
    pub fn lazy_load_reset(
        &self,
        user_id: &UserId,
        device_id: &DeviceId,
        room_id: &RoomId,
    ) -> Result<()> {
        let mut prefix = user_id.as_bytes().to_vec();
        prefix.push(0xff);
        prefix.extend_from_slice(device_id.as_bytes());
        prefix.push(0xff);
        prefix.extend_from_slice(room_id.as_bytes());
        prefix.push(0xff);

        for (key, _) in self.lazyloadedids.scan_prefix(prefix) {
            self.lazyloadedids.remove(&key)?;
        }

        Ok(())
    }

    /// Checks if a room exists.
    #[tracing::instrument(skip(self))]
    pub fn exists(&self, room_id: &RoomId) -> Result<bool> {
        let prefix = match self.get_shortroomid(room_id)? {
            Some(b) => b.to_be_bytes().to_vec(),
            None => return Ok(false),
        };

        // Look for PDUs in that room.
        Ok(self
            .pduid_pdu
            .iter_from(&prefix, false)
            .next()
            .filter(|(k, _)| k.starts_with(&prefix))
            .is_some())
    }

    pub fn get_shortroomid(&self, room_id: &RoomId) -> Result<Option<u64>> {
        self.roomid_shortroomid
            .get(room_id.as_bytes())?
            .map(|bytes| {
                utils::u64_from_bytes(&bytes)
                    .map_err(|_| Error::bad_database("Invalid shortroomid in db."))
            })
            .transpose()
    }

    pub fn get_or_create_shortroomid(
        &self,
        room_id: &RoomId,
        globals: &super::globals::Globals,
    ) -> Result<u64> {
        Ok(match self.roomid_shortroomid.get(room_id.as_bytes())? {
            Some(short) => utils::u64_from_bytes(&short)
                .map_err(|_| Error::bad_database("Invalid shortroomid in db."))?,
            None => {
                let short = globals.next_count()?;
                self.roomid_shortroomid
                    .insert(room_id.as_bytes(), &short.to_be_bytes())?;
                short
            }
        })
    }

    /// Returns the pdu from the outlier tree.
    pub fn get_outlier_pdu_json(&self, event_id: &EventId) -> Result<Option<CanonicalJsonObject>> {
        self.eventid_outlierpdu
            .get(event_id.as_bytes())?
            .map_or(Ok(None), |pdu| {
                serde_json::from_slice(&pdu).map_err(|_| Error::bad_database("Invalid PDU in db."))
            })
    }

    /// Returns the pdu from the outlier tree.
    pub fn get_pdu_outlier(&self, event_id: &EventId) -> Result<Option<PduEvent>> {
        self.eventid_outlierpdu
            .get(event_id.as_bytes())?
            .map_or(Ok(None), |pdu| {
                serde_json::from_slice(&pdu).map_err(|_| Error::bad_database("Invalid PDU in db."))
            })
    }

    /// Append the PDU as an outlier.
    #[tracing::instrument(skip(self, pdu))]
    pub fn add_pdu_outlier(&self, event_id: &EventId, pdu: &CanonicalJsonObject) -> Result<()> {
        self.eventid_outlierpdu.insert(
            event_id.as_bytes(),
            &serde_json::to_vec(&pdu).expect("CanonicalJsonObject is valid"),
        )
    }


    #[tracing::instrument(skip(self, room_id, event_ids))]
    pub fn mark_as_referenced(&self, room_id: &RoomId, event_ids: &[Arc<EventId>]) -> Result<()> {
        for prev in event_ids {
            let mut key = room_id.as_bytes().to_vec();
            key.extend_from_slice(prev.as_bytes());
            self.referencedevents.insert(&key, &[])?;
        }

        Ok(())
    }

    #[tracing::instrument(skip(self))]
    pub fn is_event_referenced(&self, room_id: &RoomId, event_id: &EventId) -> Result<bool> {
        let mut key = room_id.as_bytes().to_vec();
        key.extend_from_slice(event_id.as_bytes());
        Ok(self.referencedevents.get(&key)?.is_some())
    }

    #[tracing::instrument(skip(self))]
    pub fn mark_event_soft_failed(&self, event_id: &EventId) -> Result<()> {
        self.softfailedeventids.insert(event_id.as_bytes(), &[])
    }

    #[tracing::instrument(skip(self))]
    pub fn is_event_soft_failed(&self, event_id: &EventId) -> Result<bool> {
        self.softfailedeventids
            .get(event_id.as_bytes())
            .map(|o| o.is_some())
    }

