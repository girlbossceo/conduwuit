use super::{PduCount, RawPduId};
use crate::utils::u64_from_u8x8;

pub type ShortRoomId = ShortId;
pub type ShortEventId = ShortId;
pub type ShortId = u64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PduId {
	pub shortroomid: ShortRoomId,
	pub shorteventid: PduCount,
}

impl From<RawPduId> for PduId {
	#[inline]
	fn from(raw: RawPduId) -> Self {
		Self {
			shortroomid: u64_from_u8x8(raw.shortroomid()),
			shorteventid: PduCount::from_unsigned(u64_from_u8x8(raw.shorteventid())),
		}
	}
}
