use arrayvec::ArrayVec;

use super::{PduCount, PduId, ShortEventId, ShortId, ShortRoomId};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RawPduId {
	Normal(RawPduIdNormal),
	Backfilled(RawPduIdBackfilled),
}

type RawPduIdNormal = [u8; RawPduId::NORMAL_LEN];
type RawPduIdBackfilled = [u8; RawPduId::BACKFILLED_LEN];

const INT_LEN: usize = size_of::<ShortId>();

impl RawPduId {
	const BACKFILLED_LEN: usize = size_of::<ShortRoomId>() + INT_LEN + size_of::<ShortEventId>();
	const MAX_LEN: usize = Self::BACKFILLED_LEN;
	const NORMAL_LEN: usize = size_of::<ShortRoomId>() + size_of::<ShortEventId>();

	#[inline]
	#[must_use]
	pub fn pdu_count(&self) -> PduCount {
		let id: PduId = (*self).into();
		id.shorteventid
	}

	#[inline]
	#[must_use]
	pub fn shortroomid(self) -> [u8; INT_LEN] {
		match self {
			Self::Normal(raw) => raw[0..INT_LEN]
				.try_into()
				.expect("normal raw shortroomid array from slice"),
			Self::Backfilled(raw) => raw[0..INT_LEN]
				.try_into()
				.expect("backfilled raw shortroomid array from slice"),
		}
	}

	#[inline]
	#[must_use]
	pub fn shorteventid(self) -> [u8; INT_LEN] {
		match self {
			Self::Normal(raw) => raw[INT_LEN..INT_LEN * 2]
				.try_into()
				.expect("normal raw shorteventid array from slice"),
			Self::Backfilled(raw) => raw[INT_LEN * 2..INT_LEN * 3]
				.try_into()
				.expect("backfilled raw shorteventid array from slice"),
		}
	}

	#[inline]
	#[must_use]
	pub fn as_bytes(&self) -> &[u8] {
		match self {
			Self::Normal(ref raw) => raw,
			Self::Backfilled(ref raw) => raw,
		}
	}
}

impl AsRef<[u8]> for RawPduId {
	#[inline]
	fn as_ref(&self) -> &[u8] { self.as_bytes() }
}

impl From<&[u8]> for RawPduId {
	#[inline]
	fn from(id: &[u8]) -> Self {
		match id.len() {
			Self::NORMAL_LEN => Self::Normal(
				id[0..Self::NORMAL_LEN]
					.try_into()
					.expect("normal RawPduId from [u8]"),
			),
			Self::BACKFILLED_LEN => Self::Backfilled(
				id[0..Self::BACKFILLED_LEN]
					.try_into()
					.expect("backfilled RawPduId from [u8]"),
			),
			_ => unimplemented!("unrecognized RawPduId length"),
		}
	}
}

impl From<PduId> for RawPduId {
	#[inline]
	fn from(id: PduId) -> Self {
		const MAX_LEN: usize = RawPduId::MAX_LEN;
		type RawVec = ArrayVec<u8, MAX_LEN>;

		let mut vec = RawVec::new();
		vec.extend(id.shortroomid.to_be_bytes());
		id.shorteventid.debug_assert_valid();
		match id.shorteventid {
			PduCount::Normal(shorteventid) => {
				vec.extend(shorteventid.to_be_bytes());
				Self::Normal(
					vec.as_ref()
						.try_into()
						.expect("RawVec into RawPduId::Normal"),
				)
			},
			PduCount::Backfilled(shorteventid) => {
				vec.extend(0_u64.to_be_bytes());
				vec.extend(shorteventid.to_be_bytes());
				Self::Backfilled(
					vec.as_ref()
						.try_into()
						.expect("RawVec into RawPduId::Backfilled"),
				)
			},
		}
	}
}
