use std::sync::Arc;

use ruma::{events::TimelineEventType, state_res, EventId, MilliSecondsSinceUnixEpoch, RoomId, UserId};
use serde_json::value::RawValue as RawJsonValue;

use super::PduEvent;

impl state_res::Event for PduEvent {
	type Id = Arc<EventId>;

	fn event_id(&self) -> &Self::Id { &self.event_id }

	fn room_id(&self) -> &RoomId { &self.room_id }

	fn sender(&self) -> &UserId { &self.sender }

	fn event_type(&self) -> &TimelineEventType { &self.kind }

	fn content(&self) -> &RawJsonValue { &self.content }

	fn origin_server_ts(&self) -> MilliSecondsSinceUnixEpoch { MilliSecondsSinceUnixEpoch(self.origin_server_ts) }

	fn state_key(&self) -> Option<&str> { self.state_key.as_deref() }

	fn prev_events(&self) -> impl DoubleEndedIterator<Item = &Self::Id> + Send + '_ { self.prev_events.iter() }

	fn auth_events(&self) -> impl DoubleEndedIterator<Item = &Self::Id> + Send + '_ { self.auth_events.iter() }

	fn redacts(&self) -> Option<&Self::Id> { self.redacts.as_ref() }
}
