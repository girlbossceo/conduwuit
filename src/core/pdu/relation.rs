use ruma::events::relation::RelationType;
use serde::Deserialize;

use crate::implement;

#[derive(Clone, Debug, Deserialize)]
struct ExtractRelType {
	rel_type: RelationType,
}
#[derive(Clone, Debug, Deserialize)]
struct ExtractRelatesToEventId {
	#[serde(rename = "m.relates_to")]
	relates_to: ExtractRelType,
}

#[implement(super::PduEvent)]
#[must_use]
pub fn relation_type_equal(&self, rel_type: &RelationType) -> bool {
	self.get_content()
		.map(|c: ExtractRelatesToEventId| c.relates_to.rel_type)
		.is_ok_and(|r| r == *rel_type)
}
