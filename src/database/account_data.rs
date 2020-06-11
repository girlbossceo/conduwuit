use crate::{utils, Error, Result};
use ruma::{
    api::client::error::ErrorKind,
    events::{collections::only::Event as EduEvent, EventJson, EventType},
    identifiers::{RoomId, UserId},
};
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
            return Err(Error::BadRequest(
                ErrorKind::BadJson,
                "Json needs to have a content field.",
            ));
        }
        json.insert("type".to_owned(), kind.to_string().into());

        let user_id_string = user_id.to_string();
        let kind_string = kind.to_string();

        let mut prefix = room_id
            .map(|r| r.to_string())
            .unwrap_or_default()
            .as_bytes()
            .to_vec();
        prefix.push(0xff);
        prefix.extend_from_slice(&user_id_string.as_bytes());
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
                let user = key.split(|&b| b == 0xff).nth(1);
                let k = key.rsplit(|&b| b == 0xff).next();

                user.filter(|&user| user == user_id_string.as_bytes())
                    .is_some()
                    && k.filter(|&k| k == kind_string.as_bytes()).is_some()
            })
        {
            // This is the old room_latest
            self.roomuserdataid_accountdata.remove(old)?;
        }

        let mut key = prefix;
        key.extend_from_slice(&globals.next_count()?.to_be_bytes());
        key.push(0xff);
        key.extend_from_slice(kind.to_string().as_bytes());

        self.roomuserdataid_accountdata.insert(
            key,
            &*serde_json::to_string(&json).expect("Map::to_string always works"),
        )?;

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
                    EventType::try_from(
                        utils::string_from_bytes(k.rsplit(|&b| b == 0xff).next().ok_or_else(
                            || Error::bad_database("RoomUserData ID in db is invalid."),
                        )?)
                        .map_err(|_| Error::bad_database("RoomUserData ID in db is invalid."))?,
                    )
                    .map_err(|_| Error::bad_database("RoomUserData ID in db is invalid."))?,
                    serde_json::from_slice::<EventJson<EduEvent>>(&v).map_err(|_| {
                        Error::bad_database("Database contains invalid account data.")
                    })?,
                ))
            })
        {
            let (kind, data) = r?;
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
