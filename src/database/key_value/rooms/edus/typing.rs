use std::collections::HashSet;
use std::mem;

use ruma::{OwnedUserId, RoomId, UserId};

use crate::{database::KeyValueDatabase, service, services, utils, Error, Result};

impl service::rooms::edus::typing::Data for KeyValueDatabase {
    fn typing_add(&self, user_id: &UserId, room_id: &RoomId, timeout: u64) -> Result<()> {
        let mut prefix = room_id.as_bytes().to_vec();
        prefix.push(0xff);

        let count = services().globals.next_count()?.to_be_bytes();

        let mut room_typing_id = prefix;
        room_typing_id.extend_from_slice(&timeout.to_be_bytes());
        room_typing_id.push(0xff);
        room_typing_id.extend_from_slice(&count);

        self.typingid_userid
            .insert(&room_typing_id, user_id.as_bytes())?;

        self.roomid_lasttypingupdate
            .insert(room_id.as_bytes(), &count)?;

        Ok(())
    }

    fn typing_remove(&self, user_id: &UserId, room_id: &RoomId) -> Result<()> {
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
            self.roomid_lasttypingupdate.insert(
                room_id.as_bytes(),
                &services().globals.next_count()?.to_be_bytes(),
            )?;
        }

        Ok(())
    }

    fn typings_maintain(
        &self,
        room_id: &RoomId,
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
                .insert(room_id.as_bytes(), &services().globals.next_count()?.to_be_bytes())?;
        }

        Ok(())
    }

    fn last_typing_update(&self, room_id: &RoomId) -> Result<u64> {
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

    fn typings_all(&self, room_id: &RoomId) -> Result<HashSet<OwnedUserId>> {
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

        Ok(user_ids)
    }
}
