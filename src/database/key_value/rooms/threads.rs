use std::mem;

use ruma::{api::client::threads::get_threads::v1::IncludeThreads, OwnedUserId, RoomId, UserId};
use tracing::debug;

use crate::{database::KeyValueDatabase, service, services, utils, Error, PduEvent, Result};

type PduEventIterResult<'a> = Result<Box<dyn Iterator<Item = Result<(u64, PduEvent)>> + 'a>>;

impl service::rooms::threads::Data for KeyValueDatabase {
    fn threads_until<'a>(
        &'a self,
        user_id: &'a UserId,
        room_id: &'a RoomId,
        until: u64,
        _include: &'a IncludeThreads,
    ) -> PduEventIterResult<'a> {
        let prefix = services()
            .rooms
            .short
            .get_shortroomid(room_id)?
            .expect("room exists")
            .to_be_bytes()
            .to_vec();

        let mut current = prefix.clone();
        current.extend_from_slice(&(until - 1).to_be_bytes());

        Ok(Box::new(
            self.threadid_userids
                .iter_from(&current, true)
                .take_while(move |(k, _)| k.starts_with(&prefix))
                .map(move |(pduid, _users)| {
                    let count = utils::u64_from_bytes(&pduid[(mem::size_of::<u64>())..])
                        .map_err(|_| Error::bad_database("Invalid pduid in threadid_userids."))?;
                    let mut pdu = services()
                        .rooms
                        .timeline
                        .get_pdu_from_id(&pduid)?
                        .ok_or_else(|| {
                            Error::bad_database("Invalid pduid reference in threadid_userids")
                        })?;
                    if pdu.sender != user_id {
                        pdu.remove_transaction_id()?;
                    }
                    Ok((count, pdu))
                }),
        ))
    }

    fn delete_all_rooms_threads(&self, room_id: &RoomId) -> Result<()> {
        let mut prefix = room_id.as_bytes().to_vec();
        prefix.push(0xff);

        for (key, _) in self.threadid_userids.scan_prefix(prefix) {
            debug!("Removing key: {:?}", key);
            self.threadid_userids.remove(&key)?;
        }

        Ok(())
    }

    fn update_participants(&self, root_id: &[u8], participants: &[OwnedUserId]) -> Result<()> {
        let users = participants
            .iter()
            .map(|user| user.as_bytes())
            .collect::<Vec<_>>()
            .join(&[0xff][..]);

        self.threadid_userids.insert(root_id, &users)?;

        Ok(())
    }

    fn get_participants(&self, root_id: &[u8]) -> Result<Option<Vec<OwnedUserId>>> {
        if let Some(users) = self.threadid_userids.get(root_id)? {
            Ok(Some(
                users
                    .split(|b| *b == 0xff)
                    .map(|bytes| {
                        UserId::parse(utils::string_from_bytes(bytes).map_err(|_| {
                            Error::bad_database("Invalid UserId bytes in threadid_userids.")
                        })?)
                        .map_err(|_| Error::bad_database("Invalid UserId in threadid_userids."))
                    })
                    .filter_map(|r| r.ok())
                    .collect(),
            ))
        } else {
            Ok(None)
        }
    }
}
