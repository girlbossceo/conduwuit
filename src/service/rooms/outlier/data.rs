use std::sync::Arc;

use conduit::{Error, Result};
use database::{Database, Map};
use ruma::{CanonicalJsonObject, EventId};

use crate::PduEvent;

pub(super) struct Data {
	eventid_outlierpdu: Arc<Map>,
}

impl Data {
	pub(super) fn new(db: &Arc<Database>) -> Self {
		Self {
			eventid_outlierpdu: db["eventid_outlierpdu"].clone(),
		}
	}

	pub(super) fn get_outlier_pdu_json(&self, event_id: &EventId) -> Result<Option<CanonicalJsonObject>> {
		self.eventid_outlierpdu
			.get(event_id.as_bytes())?
			.map_or(Ok(None), |pdu| {
				serde_json::from_slice(&pdu).map_err(|_| Error::bad_database("Invalid PDU in db."))
			})
	}

	pub(super) fn get_outlier_pdu(&self, event_id: &EventId) -> Result<Option<PduEvent>> {
		self.eventid_outlierpdu
			.get(event_id.as_bytes())?
			.map_or(Ok(None), |pdu| {
				serde_json::from_slice(&pdu).map_err(|_| Error::bad_database("Invalid PDU in db."))
			})
	}

	pub(super) fn add_pdu_outlier(&self, event_id: &EventId, pdu: &CanonicalJsonObject) -> Result<()> {
		self.eventid_outlierpdu.insert(
			event_id.as_bytes(),
			&serde_json::to_vec(&pdu).expect("CanonicalJsonObject is valid"),
		)
	}
}
