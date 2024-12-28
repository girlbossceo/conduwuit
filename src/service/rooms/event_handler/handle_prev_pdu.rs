use std::{
	collections::{BTreeMap, HashMap},
	sync::Arc,
	time::Instant,
};

use conduwuit::{
	debug, implement, utils::math::continue_exponential_backoff_secs, Err, PduEvent, Result,
};
use ruma::{CanonicalJsonValue, EventId, OwnedEventId, RoomId, ServerName};

#[implement(super::Service)]
#[allow(clippy::type_complexity)]
#[allow(clippy::too_many_arguments)]
#[tracing::instrument(
	skip(self, origin, event_id, room_id, eventid_info, create_event, first_pdu_in_room),
	name = "prev"
)]
pub(super) async fn handle_prev_pdu<'a>(
	&self,
	origin: &'a ServerName,
	event_id: &'a EventId,
	room_id: &'a RoomId,
	eventid_info: &mut HashMap<
		OwnedEventId,
		(Arc<PduEvent>, BTreeMap<String, CanonicalJsonValue>),
	>,
	create_event: &Arc<PduEvent>,
	first_pdu_in_room: &Arc<PduEvent>,
	prev_id: &EventId,
) -> Result {
	// Check for disabled again because it might have changed
	if self.services.metadata.is_disabled(room_id).await {
		return Err!(Request(Forbidden(debug_warn!(
			"Federaton of room {room_id} is currently disabled on this server. Request by \
			 origin {origin} and event ID {event_id}"
		))));
	}

	if let Some((time, tries)) = self
		.services
		.globals
		.bad_event_ratelimiter
		.read()
		.expect("locked")
		.get(prev_id)
	{
		// Exponential backoff
		const MIN_DURATION: u64 = 5 * 60;
		const MAX_DURATION: u64 = 60 * 60 * 24;
		if continue_exponential_backoff_secs(MIN_DURATION, MAX_DURATION, time.elapsed(), *tries) {
			debug!(
				?tries,
				duration = ?time.elapsed(),
				"Backing off from prev_event"
			);
			return Ok(());
		}
	}

	if let Some((pdu, json)) = eventid_info.remove(prev_id) {
		// Skip old events
		if pdu.origin_server_ts < first_pdu_in_room.origin_server_ts {
			return Ok(());
		}

		let start_time = Instant::now();
		self.federation_handletime
			.write()
			.expect("locked")
			.insert(room_id.to_owned(), ((*prev_id).to_owned(), start_time));

		self.upgrade_outlier_to_timeline_pdu(pdu, json, create_event, origin, room_id)
			.await?;

		self.federation_handletime
			.write()
			.expect("locked")
			.remove(&room_id.to_owned());

		debug!(
			elapsed = ?start_time.elapsed(),
			"Handled prev_event",
		);
	}

	Ok(())
}
