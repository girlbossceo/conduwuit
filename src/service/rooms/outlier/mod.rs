mod data;

pub(crate) use data::Data;
use ruma::{CanonicalJsonObject, EventId};

use crate::{PduEvent, Result};

pub(crate) struct Service {
	pub(crate) db: &'static dyn Data,
}

impl Service {
	/// Returns the pdu from the outlier tree.
	pub(crate) fn get_outlier_pdu_json(&self, event_id: &EventId) -> Result<Option<CanonicalJsonObject>> {
		self.db.get_outlier_pdu_json(event_id)
	}

	/// Returns the pdu from the outlier tree.
	pub(crate) fn get_pdu_outlier(&self, event_id: &EventId) -> Result<Option<PduEvent>> {
		self.db.get_outlier_pdu(event_id)
	}

	/// Append the PDU as an outlier.
	#[tracing::instrument(skip(self, pdu))]
	pub(crate) fn add_pdu_outlier(&self, event_id: &EventId, pdu: &CanonicalJsonObject) -> Result<()> {
		self.db.add_pdu_outlier(event_id, pdu)
	}
}
