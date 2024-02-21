use std::sync::Arc;

use crate::{
    service::rooms::timeline::{data::PduData, PduCount},
    Result,
};
use ruma::{EventId, RoomId, UserId};

pub trait Data: Send + Sync {
    fn add_relation(&self, from: u64, to: u64) -> Result<()>;
    fn relations_until<'a>(
        &'a self,
        user_id: &'a UserId,
        room_id: u64,
        target: u64,
        until: PduCount,
    ) -> PduData<'a>;
    fn mark_as_referenced(&self, room_id: &RoomId, event_ids: &[Arc<EventId>]) -> Result<()>;
    fn delete_all_referenced_for_room(&self, room_id: &RoomId) -> Result<()>;
    fn is_event_referenced(&self, room_id: &RoomId, event_id: &EventId) -> Result<bool>;
    fn mark_event_soft_failed(&self, event_id: &EventId) -> Result<()>;
    fn is_event_soft_failed(&self, event_id: &EventId) -> Result<bool>;
}
