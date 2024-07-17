mod data;

use std::sync::Arc;

use conduit::Result;
use data::Data;
use ruma::{CanonicalJsonObject, EventId};

use crate::PduEvent;

pub struct Service {
	db: Data,
}

impl crate::Service for Service {
	fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
		Ok(Arc::new(Self {
			db: Data::new(args.db),
		}))
	}

	fn name(&self) -> &str { crate::service::make_name(std::module_path!()) }
}

impl Service {
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
	#[tracing::instrument(skip(self, pdu), level = "debug")]
	pub fn add_pdu_outlier(&self, event_id: &EventId, pdu: &CanonicalJsonObject) -> Result<()> {
		self.db.add_pdu_outlier(event_id, pdu)
	}
}
