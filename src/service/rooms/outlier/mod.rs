use conduit::Server;
use database::KeyValueDatabase;

mod data;

use std::sync::Arc;

use data::Data;
use ruma::{CanonicalJsonObject, EventId};

use crate::{PduEvent, Result};

pub struct Service {
	pub db: Data,
}

impl Service {
	pub fn build(_server: &Arc<Server>, db: &Arc<KeyValueDatabase>) -> Result<Self> {
		Ok(Self {
			db: Data::new(db),
		})
	}

	/// Returns the pdu from the outlier tree.
	pub fn get_outlier_pdu_json(&self, event_id: &EventId) -> Result<Option<CanonicalJsonObject>> {
		self.db.get_outlier_pdu_json(event_id)
	}

	/// Returns the pdu from the outlier tree.
	///
	/// TODO: use this?
	#[allow(dead_code)]
	pub fn get_pdu_outlier(&self, event_id: &EventId) -> Result<Option<PduEvent>> { self.db.get_outlier_pdu(event_id) }

	/// Append the PDU as an outlier.
	#[tracing::instrument(skip(self, pdu))]
	pub fn add_pdu_outlier(&self, event_id: &EventId, pdu: &CanonicalJsonObject) -> Result<()> {
		self.db.add_pdu_outlier(event_id, pdu)
	}
}
