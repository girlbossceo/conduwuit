use crate::Result;
use ruma_events::{collections::only::Event as EduEvent, EventJson};
use ruma_identifiers::UserId;

pub struct GlobalEdus {
    //pub globalallid_globalall: sled::Tree, // ToDevice, GlobalAllId = UserId + Count
    pub(super) globallatestid_globallatest: sled::Tree, // Presence, GlobalLatestId = Count + UserId
}

impl GlobalEdus {
    /// Adds a global event which will be saved until a new event replaces it (e.g. presence updates).
    pub fn update_globallatest(
        &self,
        user_id: &UserId,
        event: EduEvent,
        globals: &super::globals::Globals,
    ) -> Result<()> {
        // Remove old entry
        if let Some(old) = self
            .globallatestid_globallatest
            .iter()
            .keys()
            .rev()
            .filter_map(|r| r.ok())
            .find(|key| {
                key.rsplit(|&b| b == 0xff).next().unwrap() == user_id.to_string().as_bytes()
            })
        {
            // This is the old global_latest
            self.globallatestid_globallatest.remove(old)?;
        }

        let mut global_latest_id = globals.next_count()?.to_be_bytes().to_vec();
        global_latest_id.push(0xff);
        global_latest_id.extend_from_slice(&user_id.to_string().as_bytes());

        self.globallatestid_globallatest
            .insert(global_latest_id, &*serde_json::to_string(&event)?)?;

        Ok(())
    }

    /// Returns an iterator over the most recent presence updates that happened after the event with id `since`.
    pub fn globallatests_since(
        &self,
        since: u64,
    ) -> Result<impl Iterator<Item = Result<EventJson<EduEvent>>>> {
        let first_possible_edu = since.to_be_bytes().to_vec();

        Ok(self
            .globallatestid_globallatest
            .range(&*first_possible_edu..)
            // Skip the first pdu if it's exactly at since, because we sent that last time
            .skip(
                if self
                    .globallatestid_globallatest
                    .get(first_possible_edu)?
                    .is_some()
                {
                    1
                } else {
                    0
                },
            )
            .filter_map(|r| r.ok())
            .map(|(_, v)| Ok(serde_json::from_slice(&v)?)))
    }
}
