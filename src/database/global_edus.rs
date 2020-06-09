use crate::{Error, Result};
use ruma::events::EventJson;

pub struct GlobalEdus {
    //pub globalallid_globalall: sled::Tree, // ToDevice, GlobalAllId = UserId + Count
    pub(super) presenceid_presence: sled::Tree, // Presence, PresenceId = Count + UserId
}

impl GlobalEdus {
    /// Adds a global event which will be saved until a new event replaces it (e.g. presence updates).
    pub fn update_presence(
        &self,
        presence: ruma::events::presence::PresenceEvent,
        globals: &super::globals::Globals,
    ) -> Result<()> {
        // Remove old entry
        if let Some(old) = self
            .presenceid_presence
            .iter()
            .keys()
            .rev()
            .filter_map(|r| r.ok())
            .find(|key| {
                key.rsplit(|&b| b == 0xff)
                    .next()
                    .expect("rsplit always returns an element")
                    == presence.sender.to_string().as_bytes()
            })
        {
            // This is the old global_latest
            self.presenceid_presence.remove(old)?;
        }

        let mut presence_id = globals.next_count()?.to_be_bytes().to_vec();
        presence_id.push(0xff);
        presence_id.extend_from_slice(&presence.sender.to_string().as_bytes());

        self.presenceid_presence.insert(
            presence_id,
            &*serde_json::to_string(&presence).expect("PresenceEvent can be serialized"),
        )?;

        Ok(())
    }

    /// Returns an iterator over the most recent presence updates that happened after the event with id `since`.
    pub fn presence_since(
        &self,
        since: u64,
    ) -> Result<impl Iterator<Item = Result<EventJson<ruma::events::presence::PresenceEvent>>>>
    {
        let first_possible_edu = (since + 1).to_be_bytes().to_vec(); // +1 so we don't send the event at since

        Ok(self
            .presenceid_presence
            .range(&*first_possible_edu..)
            .filter_map(|r| r.ok())
            .map(|(_, v)| {
                Ok(serde_json::from_slice(&v)
                    .map_err(|_| Error::BadDatabase("Invalid presence event in db."))?)
            }))
    }
}
