use std::{
	collections::{hash_map, BTreeMap},
	sync::Arc,
	time::Instant,
};

use conduwuit::{debug, err, implement, warn, Error, Result};
use futures::{FutureExt, TryFutureExt};
use ruma::{
	api::client::error::ErrorKind, events::StateEventType, CanonicalJsonValue, EventId, RoomId,
	ServerName, UserId,
};

use super::{check_room_id, get_room_version_id};
use crate::rooms::timeline::RawPduId;

/// When receiving an event one needs to:
/// 0. Check the server is in the room
/// 1. Skip the PDU if we already know about it
/// 1.1. Remove unsigned field
/// 2. Check signatures, otherwise drop
/// 3. Check content hash, redact if doesn't match
/// 4. Fetch any missing auth events doing all checks listed here starting at 1.
///    These are not timeline events
/// 5. Reject "due to auth events" if can't get all the auth events or some of
///    the auth events are also rejected "due to auth events"
/// 6. Reject "due to auth events" if the event doesn't pass auth based on the
///    auth events
/// 7. Persist this event as an outlier
/// 8. If not timeline event: stop
/// 9. Fetch any missing prev events doing all checks listed here starting at 1.
///    These are timeline events
/// 10. Fetch missing state and auth chain events by calling `/state_ids` at
///     backwards extremities doing all the checks in this list starting at
///     1. These are not timeline events
/// 11. Check the auth of the event passes based on the state of the event
/// 12. Ensure that the state is derived from the previous current state (i.e.
///     we calculated by doing state res where one of the inputs was a
///     previously trusted set of state, don't just trust a set of state we got
///     from a remote)
/// 13. Use state resolution to find new room state
/// 14. Check if the event passes auth based on the "current state" of the room,
///     if not soft fail it
#[implement(super::Service)]
#[tracing::instrument(skip(self, origin, value, is_timeline_event), name = "pdu")]
pub async fn handle_incoming_pdu<'a>(
	&self,
	origin: &'a ServerName,
	room_id: &'a RoomId,
	event_id: &'a EventId,
	value: BTreeMap<String, CanonicalJsonValue>,
	is_timeline_event: bool,
) -> Result<Option<RawPduId>> {
	// 1. Skip the PDU if we already have it as a timeline event
	if let Ok(pdu_id) = self.services.timeline.get_pdu_id(event_id).await {
		return Ok(Some(pdu_id));
	}

	// 1.1 Check the server is in the room
	if !self.services.metadata.exists(room_id).await {
		return Err(Error::BadRequest(ErrorKind::NotFound, "Room is unknown to this server"));
	}

	// 1.2 Check if the room is disabled
	if self.services.metadata.is_disabled(room_id).await {
		return Err(Error::BadRequest(
			ErrorKind::forbidden(),
			"Federation of this room is currently disabled on this server.",
		));
	}

	// 1.3.1 Check room ACL on origin field/server
	self.acl_check(origin, room_id).await?;

	// 1.3.2 Check room ACL on sender's server name
	let sender: &UserId = value
		.get("sender")
		.try_into()
		.map_err(|e| err!(Request(InvalidParam("PDU does not have a valid sender key: {e}"))))?;

	self.acl_check(sender.server_name(), room_id).await?;

	// Fetch create event
	let create_event = self
		.services
		.state_accessor
		.room_state_get(room_id, &StateEventType::RoomCreate, "")
		.map_ok(Arc::new)
		.await?;

	// Procure the room version
	let room_version_id = get_room_version_id(&create_event)?;

	let first_pdu_in_room = self.services.timeline.first_pdu_in_room(room_id).await?;

	let (incoming_pdu, val) = self
		.handle_outlier_pdu(origin, &create_event, event_id, room_id, value, false)
		.boxed()
		.await?;

	check_room_id(room_id, &incoming_pdu)?;

	// 8. if not timeline event: stop
	if !is_timeline_event {
		return Ok(None);
	}
	// Skip old events
	if incoming_pdu.origin_server_ts < first_pdu_in_room.origin_server_ts {
		return Ok(None);
	}

	// 9. Fetch any missing prev events doing all checks listed here starting at 1.
	//    These are timeline events
	let (sorted_prev_events, mut eventid_info) = self
		.fetch_prev(
			origin,
			&create_event,
			room_id,
			&room_version_id,
			incoming_pdu.prev_events.clone(),
		)
		.await?;

	debug!(events = ?sorted_prev_events, "Got previous events");
	for prev_id in sorted_prev_events {
		self.services.server.check_running()?;
		if let Err(e) = self
			.handle_prev_pdu(
				origin,
				event_id,
				room_id,
				&mut eventid_info,
				&create_event,
				&first_pdu_in_room,
				&prev_id,
			)
			.await
		{
			use hash_map::Entry;

			let now = Instant::now();
			warn!("Prev event {prev_id} failed: {e}");

			match self
				.services
				.globals
				.bad_event_ratelimiter
				.write()
				.expect("locked")
				.entry(prev_id)
			{
				| Entry::Vacant(e) => {
					e.insert((now, 1));
				},
				| Entry::Occupied(mut e) => {
					*e.get_mut() = (now, e.get().1.saturating_add(1));
				},
			};
		}
	}

	// Done with prev events, now handling the incoming event
	let start_time = Instant::now();
	self.federation_handletime
		.write()
		.expect("locked")
		.insert(room_id.to_owned(), (event_id.to_owned(), start_time));

	let r = self
		.upgrade_outlier_to_timeline_pdu(incoming_pdu, val, &create_event, origin, room_id)
		.await;

	self.federation_handletime
		.write()
		.expect("locked")
		.remove(&room_id.to_owned());

	r
}
