use crate::{utils, Error, Result};
use ruma_events::{collections::only::Event as EduEvent, EventJson, EventType};
use ruma_identifiers::{RoomId, UserId};
use std::{collections::HashMap, convert::TryFrom};

pub struct AccountData {
    pub(super) roomuserdataid_accountdata: sled::Tree, // RoomUserDataId = Room + User + Count + Type
}

impl AccountData {
    /// Places one event in the account data of the user and removes the previous entry.
    pub fn update(
        &self,
        room_id: Option<&RoomId>,
        user_id: &UserId,
        kind: &EventType,
        json: &mut serde_json::Map<String, serde_json::Value>,
        globals: &super::globals::Globals,
    ) -> Result<()> {
        if json.get("content").is_none() {
            return Err(Error::BadRequest("json needs to have a content field"));
        }
        json.insert("type".to_owned(), kind.to_string().into());

        let mut prefix = room_id
            .map(|r| r.to_string())
            .unwrap_or_default()
            .as_bytes()
            .to_vec();
        prefix.push(0xff);
        prefix.extend_from_slice(&user_id.to_string().as_bytes());
        prefix.push(0xff);

        // Remove old entry
        if let Some(old) = self
            .roomuserdataid_accountdata
            .scan_prefix(&prefix)
            .keys()
            .rev()
            .filter_map(|r| r.ok())
            .take_while(|key| key.starts_with(&prefix))
            .find(|key| {
                key.split(|&b| b == 0xff)
                    .nth(1)
                    .filter(|&user| user == user_id.to_string().as_bytes())
                    .is_some()
            })
        {
            // This is the old room_latest
            self.roomuserdataid_accountdata.remove(old)?;
        }

        let mut key = prefix;
        key.extend_from_slice(&globals.next_count()?.to_be_bytes());
        key.push(0xff);
        key.extend_from_slice(kind.to_string().as_bytes());

        self.roomuserdataid_accountdata
            .insert(key, &*serde_json::to_string(&json)?)
            .unwrap();

        Ok(())
    }

    // TODO: Optimize
    /// Searches the account data for a specific kind.
    pub fn get(
        &self,
        room_id: Option<&RoomId>,
        user_id: &UserId,
        kind: &EventType,
    ) -> Result<Option<EventJson<EduEvent>>> {
        Ok(self.all(room_id, user_id)?.remove(kind))
    }

    /// Returns all changes to the account data that happened after `since`.
    pub fn changes_since(
        &self,
        room_id: Option<&RoomId>,
        user_id: &UserId,
        since: u64,
    ) -> Result<HashMap<EventType, EventJson<EduEvent>>> {
        let mut userdata = HashMap::new();

        let mut prefix = room_id
            .map(|r| r.to_string())
            .unwrap_or_default()
            .as_bytes()
            .to_vec();
        prefix.push(0xff);
        prefix.extend_from_slice(&user_id.to_string().as_bytes());
        prefix.push(0xff);

        // Skip the data that's exactly at since, because we sent that last time
        let mut first_possible = prefix.clone();
        first_possible.extend_from_slice(&(since + 1).to_be_bytes());

        for r in self
            .roomuserdataid_accountdata
            .range(&*first_possible..)
            .filter_map(|r| r.ok())
            .take_while(move |(k, _)| k.starts_with(&prefix))
            .map(|(k, v)| {
                Ok::<_, Error>((
                    EventType::try_from(utils::string_from_bytes(
                        k.rsplit(|&b| b == 0xff)
                            .next()
                            .ok_or(Error::BadDatabase("roomuserdataid is invalid"))?,
                    )?)
                    .map_err(|_| Error::BadDatabase("roomuserdataid is invalid"))?,
                    serde_json::from_slice::<EventJson<EduEvent>>(&v).unwrap(),
                ))
            })
        {
            let (kind, data) = r.unwrap();
            userdata.insert(kind, data);
        }

        Ok(userdata)
    }

    /// Returns all account data.
    pub fn all(
        &self,
        room_id: Option<&RoomId>,
        user_id: &UserId,
    ) -> Result<HashMap<EventType, EventJson<EduEvent>>> {
        self.changes_since(room_id, user_id, 0)
    }
}
