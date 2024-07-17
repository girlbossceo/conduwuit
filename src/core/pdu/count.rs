use std::cmp::Ordering;

use ruma::api::client::error::ErrorKind;

use crate::{Error, Result};

#[derive(Hash, PartialEq, Eq, Clone, Copy, Debug)]
pub enum PduCount {
	Backfilled(u64),
	Normal(u64),
}

impl PduCount {
	#[must_use]
	pub fn min() -> Self { Self::Backfilled(u64::MAX) }

	#[must_use]
	pub fn max() -> Self { Self::Normal(u64::MAX) }

	pub fn try_from_string(token: &str) -> Result<Self> {
		if let Some(stripped_token) = token.strip_prefix('-') {
			stripped_token.parse().map(PduCount::Backfilled)
		} else {
			token.parse().map(PduCount::Normal)
		}
		.map_err(|_| Error::BadRequest(ErrorKind::InvalidParam, "Invalid pagination token."))
	}

	#[must_use]
	pub fn stringify(&self) -> String {
		match self {
			Self::Backfilled(x) => format!("-{x}"),
			Self::Normal(x) => x.to_string(),
		}
	}
}

impl PartialOrd for PduCount {
	fn partial_cmp(&self, other: &Self) -> Option<Ordering> { Some(self.cmp(other)) }
}

impl Ord for PduCount {
	fn cmp(&self, other: &Self) -> Ordering {
		match (self, other) {
			(Self::Normal(s), Self::Normal(o)) => s.cmp(o),
			(Self::Backfilled(s), Self::Backfilled(o)) => o.cmp(s),
			(Self::Normal(_), Self::Backfilled(_)) => Ordering::Greater,
			(Self::Backfilled(_), Self::Normal(_)) => Ordering::Less,
		}
	}
}
