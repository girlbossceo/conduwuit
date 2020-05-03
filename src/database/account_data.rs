use crate::Result;
use ruma_events::{collections::only::Event as EduEvent, EventJson};
use ruma_identifiers::{RoomId, UserId};
use std::collections::HashMap;

pub struct AccountData {
    pub(super) roomuserdataid_accountdata: sled::Tree, // RoomUserDataId = Room + User + Count + Type
}

impl AccountData {
    /// Places one event in the account data of the user and removes the previous entry.
    pub fn update(
        &self,
        room_id: Option<&RoomId>,
        user_id: &UserId,
        event: EduEvent,
        globals: &super::globals::Globals,
    ) -> Result<()> {
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
            .filter(|key| {
                key.split(|&b| b == 0xff)
                    .nth(1)
                    .filter(|&user| user == user_id.to_string().as_bytes())
                    .is_some()
            })
            .next()
        {
            // This is the old room_latest
            self.roomuserdataid_accountdata.remove(old)?;
            println!("removed old account data");
        }

        let mut key = prefix;
        key.extend_from_slice(&globals.next_count()?.to_be_bytes());
        key.push(0xff);
        let json = serde_json::to_value(&event)?;
        key.extend_from_slice(json["type"].as_str().unwrap().as_bytes());

        self.roomuserdataid_accountdata
            .insert(key, &*json.to_string())
            .unwrap();

        Ok(())
    }

    // TODO: Optimize
    /// Searches the account data for a specific kind.
    pub fn get(
        &self,
        room_id: Option<&RoomId>,
        user_id: &UserId,
        kind: &str,
    ) -> Result<Option<EventJson<EduEvent>>> {
        Ok(self.all(room_id, user_id)?.remove(kind))
    }

    /// Returns all changes to the account data that happened after `since`.
    pub fn changes_since(
        &self,
        room_id: Option<&RoomId>,
        user_id: &UserId,
        since: u64,
    ) -> Result<HashMap<String, EventJson<EduEvent>>> {
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

        for json in self
            .roomuserdataid_accountdata
            .range(&*first_possible..)
            .filter_map(|r| r.ok())
            .take_while(move |(k, _)| k.starts_with(&prefix))
            .map(|(_, v)| serde_json::from_slice::<serde_json::Value>(&v).unwrap())
        {
            userdata.insert(
                json["type"].as_str().unwrap().to_owned(),
                serde_json::from_value::<EventJson<EduEvent>>(json)
                    .expect("userdata in db is valid"),
            );
        }

        Ok(userdata)
    }

    /// Returns all account data.
    pub fn all(
        &self,
        room_id: Option<&RoomId>,
        user_id: &UserId,
    ) -> Result<HashMap<String, EventJson<EduEvent>>> {
        self.changes_since(room_id, user_id, 0)
    }
}
