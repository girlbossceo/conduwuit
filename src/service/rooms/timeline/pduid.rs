use crate::rooms::short::{ShortEventId, ShortRoomId};

#[derive(Clone, Copy)]
pub struct PduId {
	_room_id: ShortRoomId,
	_event_id: ShortEventId,
}

pub type RawPduId = [u8; PduId::LEN];

impl PduId {
	pub const LEN: usize = size_of::<ShortRoomId>() + size_of::<ShortEventId>();
}
