use conduwuit::{error, implement, utils::stream::ReadyExt};
use futures::StreamExt;
use ruma::{
	events::{
		room::history_visibility::{HistoryVisibility, RoomHistoryVisibilityEventContent},
		StateEventType,
	},
	EventId, RoomId, ServerName,
};

/// Whether a server is allowed to see an event through federation, based on
/// the room's history_visibility at that event's state.
#[implement(super::Service)]
#[tracing::instrument(skip_all, level = "trace")]
pub async fn server_can_see_event(
	&self,
	origin: &ServerName,
	room_id: &RoomId,
	event_id: &EventId,
) -> bool {
	let Ok(shortstatehash) = self.pdu_shortstatehash(event_id).await else {
		return true;
	};

	if let Some(visibility) = self
		.server_visibility_cache
		.lock()
		.expect("locked")
		.get_mut(&(origin.to_owned(), shortstatehash))
	{
		return *visibility;
	}

	let history_visibility = self
		.state_get_content(shortstatehash, &StateEventType::RoomHistoryVisibility, "")
		.await
		.map_or(HistoryVisibility::Shared, |c: RoomHistoryVisibilityEventContent| {
			c.history_visibility
		});

	let current_server_members = self
		.services
		.state_cache
		.room_members(room_id)
		.ready_filter(|member| member.server_name() == origin);

	let visibility = match history_visibility {
		| HistoryVisibility::WorldReadable | HistoryVisibility::Shared => true,
		| HistoryVisibility::Invited => {
			// Allow if any member on requesting server was AT LEAST invited, else deny
			current_server_members
				.any(|member| self.user_was_invited(shortstatehash, member))
				.await
		},
		| HistoryVisibility::Joined => {
			// Allow if any member on requested server was joined, else deny
			current_server_members
				.any(|member| self.user_was_joined(shortstatehash, member))
				.await
		},
		| _ => {
			error!("Unknown history visibility {history_visibility}");
			false
		},
	};

	self.server_visibility_cache
		.lock()
		.expect("locked")
		.insert((origin.to_owned(), shortstatehash), visibility);

	visibility
}
