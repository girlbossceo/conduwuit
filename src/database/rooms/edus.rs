use crate::{utils, Result};
use ruma_events::{collections::only::Event as EduEvent, EventJson};
use ruma_identifiers::{RoomId, UserId};

pub struct RoomEdus {
    pub(in super::super) roomuserid_lastread: sled::Tree, // RoomUserId = Room + User
    pub(in super::super) roomlatestid_roomlatest: sled::Tree, // Read Receipts, RoomLatestId = RoomId + Count + UserId
    pub(in super::super) roomactiveid_roomactive: sled::Tree, // Typing, RoomActiveId = RoomId + TimeoutTime + Count
}

impl RoomEdus {
    /// Adds an event which will be saved until a new event replaces it (e.g. read receipt).
    pub fn roomlatest_update(
        &self,
        user_id: &UserId,
        room_id: &RoomId,
        event: EduEvent,
        globals: &super::super::globals::Globals,
    ) -> Result<()> {
        let mut prefix = room_id.to_string().as_bytes().to_vec();
        prefix.push(0xff);

        // Remove old entry
        if let Some(old) = self
            .roomlatestid_roomlatest
            .scan_prefix(&prefix)
            .keys()
            .rev()
            .filter_map(|r| r.ok())
            .take_while(|key| key.starts_with(&prefix))
            .find(|key| {
                key.rsplit(|&b| b == 0xff).next().unwrap() == user_id.to_string().as_bytes()
            })
        {
            // This is the old room_latest
            self.roomlatestid_roomlatest.remove(old)?;
        }

        let mut room_latest_id = prefix;
        room_latest_id.extend_from_slice(&globals.next_count()?.to_be_bytes());
        room_latest_id.push(0xff);
        room_latest_id.extend_from_slice(&user_id.to_string().as_bytes());

        self.roomlatestid_roomlatest
            .insert(room_latest_id, &*serde_json::to_string(&event)?)?;

        Ok(())
    }

    /// Returns an iterator over the most recent read_receipts in a room that happened after the event with id `since`.
    pub fn roomlatests_since(
        &self,
        room_id: &RoomId,
        since: u64,
    ) -> Result<impl Iterator<Item = Result<EventJson<EduEvent>>>> {
        let mut prefix = room_id.to_string().as_bytes().to_vec();
        prefix.push(0xff);

        let mut first_possible_edu = prefix.clone();
        first_possible_edu.extend_from_slice(&since.to_be_bytes());

        Ok(self
            .roomlatestid_roomlatest
            .range(&*first_possible_edu..)
            // Skip the first pdu if it's exactly at since, because we sent that last time
            .skip(
                if self
                    .roomlatestid_roomlatest
                    .get(first_possible_edu)?
                    .is_some()
                {
                    1
                } else {
                    0
                },
            )
            .filter_map(|r| r.ok())
            .take_while(move |(k, _)| k.starts_with(&prefix))
            .map(|(_, v)| Ok(serde_json::from_slice(&v)?)))
    }

    /// Returns a vector of the most recent read_receipts in a room that happened after the event with id `since`.
    pub fn roomlatests_all(
        &self,
        room_id: &RoomId,
    ) -> Result<impl Iterator<Item = Result<EventJson<EduEvent>>>> {
        self.roomlatests_since(room_id, 0)
    }

    /// Adds an event that will be saved until the `timeout` timestamp (e.g. typing notifications).
    pub fn roomactive_add(
        &self,
        event: EduEvent,
        room_id: &RoomId,
        timeout: u64,
        globals: &super::super::globals::Globals,
    ) -> Result<()> {
        let mut prefix = room_id.to_string().as_bytes().to_vec();
        prefix.push(0xff);

        // Cleanup all outdated edus before inserting a new one
        for outdated_edu in self
            .roomactiveid_roomactive
            .scan_prefix(&prefix)
            .keys()
            .filter_map(|r| r.ok())
            .take_while(|k| {
                utils::u64_from_bytes(
                    k.split(|&c| c == 0xff)
                        .nth(1)
                        .expect("roomactive has valid timestamp and delimiters"),
                ) < utils::millis_since_unix_epoch()
            })
        {
            // This is an outdated edu (time > timestamp)
            self.roomlatestid_roomlatest.remove(outdated_edu)?;
        }

        let mut room_active_id = prefix;
        room_active_id.extend_from_slice(&timeout.to_be_bytes());
        room_active_id.push(0xff);
        room_active_id.extend_from_slice(&globals.next_count()?.to_be_bytes());

        self.roomactiveid_roomactive
            .insert(room_active_id, &*serde_json::to_string(&event)?)?;

        Ok(())
    }

    /// Removes an active event manually (before the timeout is reached).
    pub fn roomactive_remove(&self, event: EduEvent, room_id: &RoomId) -> Result<()> {
        let mut prefix = room_id.to_string().as_bytes().to_vec();
        prefix.push(0xff);

        let json = serde_json::to_string(&event)?;

        // Remove outdated entries
        for outdated_edu in self
            .roomactiveid_roomactive
            .scan_prefix(&prefix)
            .filter_map(|r| r.ok())
            .filter(|(_, v)| v == json.as_bytes())
        {
            self.roomactiveid_roomactive.remove(outdated_edu.0)?;
        }

        Ok(())
    }

    /// Returns an iterator over all active events (e.g. typing notifications).
    pub fn roomactives_all(
        &self,
        room_id: &RoomId,
    ) -> impl Iterator<Item = Result<EventJson<EduEvent>>> {
        let mut prefix = room_id.to_string().as_bytes().to_vec();
        prefix.push(0xff);

        let mut first_active_edu = prefix.clone();
        first_active_edu.extend_from_slice(&utils::millis_since_unix_epoch().to_be_bytes());

        self.roomactiveid_roomactive
            .range(first_active_edu..)
            .filter_map(|r| r.ok())
            .take_while(move |(k, _)| k.starts_with(&prefix))
            .map(|(_, v)| Ok(serde_json::from_slice(&v)?))
    }

    /// Sets a private read marker at `count`.
    pub fn room_read_set(&self, room_id: &RoomId, user_id: &UserId, count: u64) -> Result<()> {
        let mut key = room_id.to_string().as_bytes().to_vec();
        key.push(0xff);
        key.extend_from_slice(&user_id.to_string().as_bytes());

        self.roomuserid_lastread.insert(key, &count.to_be_bytes())?;

        Ok(())
    }

    /// Returns the private read marker.
    pub fn room_read_get(&self, room_id: &RoomId, user_id: &UserId) -> Result<Option<u64>> {
        let mut key = room_id.to_string().as_bytes().to_vec();
        key.push(0xff);
        key.extend_from_slice(&user_id.to_string().as_bytes());

        Ok(self
            .roomuserid_lastread
            .get(key)?
            .map(|v| utils::u64_from_bytes(&v)))
    }
}
