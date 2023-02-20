use std::{collections::hash_map, mem::size_of, sync::Arc};

use ruma::{
    api::client::error::ErrorKind, CanonicalJsonObject, EventId, OwnedUserId, RoomId, UserId,
};
use tracing::error;

use crate::{database::KeyValueDatabase, service, services, utils, Error, PduEvent, Result};

use service::rooms::timeline::PduCount;

impl service::rooms::timeline::Data for KeyValueDatabase {
    fn last_timeline_count(&self, sender_user: &UserId, room_id: &RoomId) -> Result<PduCount> {
        match self
            .lasttimelinecount_cache
            .lock()
            .unwrap()
            .entry(room_id.to_owned())
        {
            hash_map::Entry::Vacant(v) => {
                if let Some(last_count) = self
                    .pdus_until(sender_user, room_id, PduCount::max())?
                    .find_map(|r| {
                        // Filter out buggy events
                        if r.is_err() {
                            error!("Bad pdu in pdus_since: {:?}", r);
                        }
                        r.ok()
                    })
                {
                    Ok(*v.insert(last_count.0))
                } else {
                    Ok(PduCount::Normal(0))
                }
            }
            hash_map::Entry::Occupied(o) => Ok(*o.get()),
        }
    }

    /// Returns the `count` of this pdu's id.
    fn get_pdu_count(&self, event_id: &EventId) -> Result<Option<PduCount>> {
        Ok(self
            .eventid_pduid
            .get(event_id.as_bytes())?
            .map(|pdu_id| Ok::<_, Error>(PduCount::Normal(pdu_count(&pdu_id)?)))
            .transpose()?
            .map_or_else(
                || {
                    Ok::<_, Error>(
                        self.eventid_backfillpduid
                            .get(event_id.as_bytes())?
                            .map(|pdu_id| Ok::<_, Error>(PduCount::Backfilled(pdu_count(&pdu_id)?)))
                            .transpose()?,
                    )
                },
                |x| Ok(Some(x)),
            )?)
    }

    /// Returns the json of a pdu.
    fn get_pdu_json(&self, event_id: &EventId) -> Result<Option<CanonicalJsonObject>> {
        self.get_non_outlier_pdu_json(event_id)?.map_or_else(
            || {
                self.eventid_outlierpdu
                    .get(event_id.as_bytes())?
                    .map(|pdu| {
                        serde_json::from_slice(&pdu)
                            .map_err(|_| Error::bad_database("Invalid PDU in db."))
                    })
                    .transpose()
            },
            |x| Ok(Some(x)),
        )
    }

    /// Returns the json of a pdu.
    fn get_non_outlier_pdu_json(&self, event_id: &EventId) -> Result<Option<CanonicalJsonObject>> {
        self.eventid_pduid
            .get(event_id.as_bytes())?
            .map(|pduid| {
                self.pduid_pdu
                    .get(&pduid)?
                    .ok_or_else(|| Error::bad_database("Invalid pduid in eventid_pduid."))
            })
            .transpose()?
            .map_or_else(
                || {
                    Ok::<_, Error>(
                        self.eventid_backfillpduid
                            .get(event_id.as_bytes())?
                            .map(|pduid| {
                                self.pduid_backfillpdu.get(&pduid)?.ok_or_else(|| {
                                    Error::bad_database("Invalid pduid in eventid_pduid.")
                                })
                            })
                            .transpose()?,
                    )
                },
                |x| Ok(Some(x)),
            )?
            .map(|pdu| {
                serde_json::from_slice(&pdu).map_err(|_| Error::bad_database("Invalid PDU in db."))
            })
            .transpose()
    }

    /// Returns the pdu's id.
    fn get_pdu_id(&self, event_id: &EventId) -> Result<Option<Vec<u8>>> {
        Ok(self.eventid_pduid.get(event_id.as_bytes())?.map_or_else(
            || self.eventid_backfillpduid.get(event_id.as_bytes()),
            |x| Ok(Some(x)),
        )?)
    }

    /// Returns the pdu.
    ///
    /// Checks the `eventid_outlierpdu` Tree if not found in the timeline.
    fn get_non_outlier_pdu(&self, event_id: &EventId) -> Result<Option<PduEvent>> {
        self.eventid_pduid
            .get(event_id.as_bytes())?
            .map(|pduid| {
                self.pduid_pdu
                    .get(&pduid)?
                    .ok_or_else(|| Error::bad_database("Invalid pduid in eventid_pduid."))
            })
            .transpose()?
            .map_or_else(
                || {
                    Ok::<_, Error>(
                        self.eventid_backfillpduid
                            .get(event_id.as_bytes())?
                            .map(|pduid| {
                                self.pduid_backfillpdu.get(&pduid)?.ok_or_else(|| {
                                    Error::bad_database("Invalid pduid in eventid_pduid.")
                                })
                            })
                            .transpose()?,
                    )
                },
                |x| Ok(Some(x)),
            )?
            .map(|pdu| {
                serde_json::from_slice(&pdu).map_err(|_| Error::bad_database("Invalid PDU in db."))
            })
            .transpose()
    }

    /// Returns the pdu.
    ///
    /// Checks the `eventid_outlierpdu` Tree if not found in the timeline.
    fn get_pdu(&self, event_id: &EventId) -> Result<Option<Arc<PduEvent>>> {
        if let Some(p) = self.pdu_cache.lock().unwrap().get_mut(event_id) {
            return Ok(Some(Arc::clone(p)));
        }

        if let Some(pdu) = self
            .get_non_outlier_pdu(event_id)?
            .map_or_else(
                || {
                    self.eventid_outlierpdu
                        .get(event_id.as_bytes())?
                        .map(|pdu| {
                            serde_json::from_slice(&pdu)
                                .map_err(|_| Error::bad_database("Invalid PDU in db."))
                        })
                        .transpose()
                },
                |x| Ok(Some(x)),
            )?
            .map(Arc::new)
        {
            self.pdu_cache
                .lock()
                .unwrap()
                .insert(event_id.to_owned(), Arc::clone(&pdu));
            Ok(Some(pdu))
        } else {
            Ok(None)
        }
    }

    /// Returns the pdu.
    ///
    /// This does __NOT__ check the outliers `Tree`.
    fn get_pdu_from_id(&self, pdu_id: &[u8]) -> Result<Option<PduEvent>> {
        self.pduid_pdu
            .get(pdu_id)?
            .map_or_else(|| self.pduid_backfillpdu.get(pdu_id), |x| Ok(Some(x)))?
            .map_or(Ok(None), |pdu| {
                Ok(Some(
                    serde_json::from_slice(&pdu)
                        .map_err(|_| Error::bad_database("Invalid PDU in db."))?,
                ))
            })
    }

    /// Returns the pdu as a `BTreeMap<String, CanonicalJsonValue>`.
    fn get_pdu_json_from_id(&self, pdu_id: &[u8]) -> Result<Option<CanonicalJsonObject>> {
        self.pduid_pdu
            .get(pdu_id)?
            .map_or_else(|| self.pduid_backfillpdu.get(pdu_id), |x| Ok(Some(x)))?
            .map_or(Ok(None), |pdu| {
                Ok(Some(
                    serde_json::from_slice(&pdu)
                        .map_err(|_| Error::bad_database("Invalid PDU in db."))?,
                ))
            })
    }

    fn append_pdu(
        &self,
        pdu_id: &[u8],
        pdu: &PduEvent,
        json: &CanonicalJsonObject,
        count: u64,
    ) -> Result<()> {
        self.pduid_pdu.insert(
            pdu_id,
            &serde_json::to_vec(json).expect("CanonicalJsonObject is always a valid"),
        )?;

        self.lasttimelinecount_cache
            .lock()
            .unwrap()
            .insert(pdu.room_id.clone(), PduCount::Normal(count));

        self.eventid_pduid.insert(pdu.event_id.as_bytes(), pdu_id)?;
        self.eventid_outlierpdu.remove(pdu.event_id.as_bytes())?;

        Ok(())
    }

    fn prepend_backfill_pdu(
        &self,
        pdu_id: &[u8],
        event_id: &EventId,
        json: &CanonicalJsonObject,
    ) -> Result<()> {
        self.pduid_backfillpdu.insert(
            pdu_id,
            &serde_json::to_vec(json).expect("CanonicalJsonObject is always a valid"),
        )?;

        self.eventid_backfillpduid
            .insert(event_id.as_bytes(), pdu_id)?;
        self.eventid_outlierpdu.remove(event_id.as_bytes())?;

        Ok(())
    }

    /// Removes a pdu and creates a new one with the same id.
    fn replace_pdu(&self, pdu_id: &[u8], pdu: &PduEvent) -> Result<()> {
        if self.pduid_pdu.get(pdu_id)?.is_some() {
            self.pduid_pdu.insert(
                pdu_id,
                &serde_json::to_vec(pdu).expect("CanonicalJsonObject is always a valid"),
            )?;
            Ok(())
        } else {
            Err(Error::BadRequest(
                ErrorKind::NotFound,
                "PDU does not exist.",
            ))
        }
    }

    /// Returns an iterator over all events and their tokens in a room that happened before the
    /// event with id `until` in reverse-chronological order.
    fn pdus_until<'a>(
        &'a self,
        user_id: &UserId,
        room_id: &RoomId,
        until: PduCount,
    ) -> Result<Box<dyn Iterator<Item = Result<(PduCount, PduEvent)>> + 'a>> {
        // Create the first part of the full pdu id
        let prefix = services()
            .rooms
            .short
            .get_shortroomid(room_id)?
            .expect("room exists")
            .to_be_bytes()
            .to_vec();

        let mut current_backfill = prefix.clone();
        // +1 so we don't send the base event
        let backfill_count = match until {
            PduCount::Backfilled(x) => x + 1,
            PduCount::Normal(_) => 0,
        };
        current_backfill.extend_from_slice(&backfill_count.to_be_bytes());

        let user_id = user_id.to_owned();
        let user_id2 = user_id.to_owned();
        let prefix2 = prefix.clone();

        let backfill_iter = self
            .pduid_backfillpdu
            .iter_from(&current_backfill, false)
            .take_while(move |(k, _)| k.starts_with(&prefix))
            .map(move |(pdu_id, v)| {
                let mut pdu = serde_json::from_slice::<PduEvent>(&v)
                    .map_err(|_| Error::bad_database("PDU in db is invalid."))?;
                if pdu.sender != user_id {
                    pdu.remove_transaction_id()?;
                }
                let count = PduCount::Backfilled(pdu_count(&pdu_id)?);
                Ok((count, pdu))
            });

        match until {
            PduCount::Backfilled(_) => Ok(Box::new(backfill_iter)),
            PduCount::Normal(x) => {
                let mut current_normal = prefix2.clone();
                // -1 so we don't send the base event
                current_normal.extend_from_slice(&x.saturating_sub(1).to_be_bytes());
                let normal_iter = self
                    .pduid_pdu
                    .iter_from(&current_normal, true)
                    .take_while(move |(k, _)| k.starts_with(&prefix2))
                    .map(move |(pdu_id, v)| {
                        let mut pdu = serde_json::from_slice::<PduEvent>(&v)
                            .map_err(|_| Error::bad_database("PDU in db is invalid."))?;
                        if pdu.sender != user_id2 {
                            pdu.remove_transaction_id()?;
                        }
                        let count = PduCount::Normal(pdu_count(&pdu_id)?);
                        Ok((count, pdu))
                    });

                Ok(Box::new(normal_iter.chain(backfill_iter)))
            }
        }
    }

    fn pdus_after<'a>(
        &'a self,
        user_id: &UserId,
        room_id: &RoomId,
        from: PduCount,
    ) -> Result<Box<dyn Iterator<Item = Result<(PduCount, PduEvent)>> + 'a>> {
        // Create the first part of the full pdu id
        let prefix = services()
            .rooms
            .short
            .get_shortroomid(room_id)?
            .expect("room exists")
            .to_be_bytes()
            .to_vec();

        let mut current_normal = prefix.clone();
        // +1 so we don't send the base event
        let normal_count = match from {
            PduCount::Normal(x) => x + 1,
            PduCount::Backfilled(_) => 0,
        };
        current_normal.extend_from_slice(&normal_count.to_be_bytes());

        let user_id = user_id.to_owned();
        let user_id2 = user_id.to_owned();
        let prefix2 = prefix.clone();

        let normal_iter = self
            .pduid_pdu
            .iter_from(&current_normal, false)
            .take_while(move |(k, _)| k.starts_with(&prefix))
            .map(move |(pdu_id, v)| {
                let mut pdu = serde_json::from_slice::<PduEvent>(&v)
                    .map_err(|_| Error::bad_database("PDU in db is invalid."))?;
                if pdu.sender != user_id {
                    pdu.remove_transaction_id()?;
                }
                let count = PduCount::Normal(pdu_count(&pdu_id)?);
                Ok((count, pdu))
            });

        match from {
            PduCount::Normal(_) => Ok(Box::new(normal_iter)),
            PduCount::Backfilled(x) => {
                let mut current_backfill = prefix2.clone();
                // -1 so we don't send the base event
                current_backfill.extend_from_slice(&x.saturating_sub(1).to_be_bytes());
                let backfill_iter = self
                    .pduid_backfillpdu
                    .iter_from(&current_backfill, true)
                    .take_while(move |(k, _)| k.starts_with(&prefix2))
                    .map(move |(pdu_id, v)| {
                        let mut pdu = serde_json::from_slice::<PduEvent>(&v)
                            .map_err(|_| Error::bad_database("PDU in db is invalid."))?;
                        if pdu.sender != user_id2 {
                            pdu.remove_transaction_id()?;
                        }
                        let count = PduCount::Backfilled(pdu_count(&pdu_id)?);
                        Ok((count, pdu))
                    });

                Ok(Box::new(backfill_iter.chain(normal_iter)))
            }
        }
    }

    fn increment_notification_counts(
        &self,
        room_id: &RoomId,
        notifies: Vec<OwnedUserId>,
        highlights: Vec<OwnedUserId>,
    ) -> Result<()> {
        let mut notifies_batch = Vec::new();
        let mut highlights_batch = Vec::new();
        for user in notifies {
            let mut userroom_id = user.as_bytes().to_vec();
            userroom_id.push(0xff);
            userroom_id.extend_from_slice(room_id.as_bytes());
            notifies_batch.push(userroom_id);
        }
        for user in highlights {
            let mut userroom_id = user.as_bytes().to_vec();
            userroom_id.push(0xff);
            userroom_id.extend_from_slice(room_id.as_bytes());
            highlights_batch.push(userroom_id);
        }

        self.userroomid_notificationcount
            .increment_batch(&mut notifies_batch.into_iter())?;
        self.userroomid_highlightcount
            .increment_batch(&mut highlights_batch.into_iter())?;
        Ok(())
    }
}

/// Returns the `count` of this pdu's id.
fn pdu_count(pdu_id: &[u8]) -> Result<u64> {
    utils::u64_from_bytes(&pdu_id[pdu_id.len() - size_of::<u64>()..])
        .map_err(|_| Error::bad_database("PDU has invalid count bytes."))
}
