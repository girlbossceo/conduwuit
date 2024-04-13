use std::{
	cmp,
	collections::{hash_map, HashSet},
	pin::Pin,
	time::{Duration, Instant},
};

use futures_util::Future;
use ruma::{
	api::{
		client::error::ErrorKind,
		federation::event::{get_event, get_room_state_ids},
	},
	events::{
		room::{create::RoomCreateEventContent, server_acl::RoomServerAclEventContent},
		StateEventType,
	},
	int,
	serde::Base64,
	state_res::{self, RoomVersion, StateMap},
	uint, CanonicalJsonValue, EventId, MilliSecondsSinceUnixEpoch, RoomId, RoomVersionId, ServerName,
};
use tokio::sync::RwLock;
use tracing::{debug, error, info, trace, warn};

use super::state_compressor::CompressedStateEvent;
use crate::{
	service::{pdu, Arc, BTreeMap, HashMap, Result},
	services, Error, PduEvent,
};

pub mod signing_keys;
pub struct Service;

// We use some AsyncRecursiveType hacks here so we can call async funtion
// recursively.
type AsyncRecursiveType<'a, T> = Pin<Box<dyn Future<Output = T> + 'a + Send>>;
type AsyncRecursiveCanonicalJsonVec<'a> =
	AsyncRecursiveType<'a, Vec<(Arc<PduEvent>, Option<BTreeMap<String, CanonicalJsonValue>>)>>;
type AsyncRecursiveCanonicalJsonResult<'a> =
	AsyncRecursiveType<'a, Result<(Arc<PduEvent>, BTreeMap<String, CanonicalJsonValue>)>>;

impl Service {
	/// When receiving an event one needs to:
	/// 0. Check the server is in the room
	/// 1. Skip the PDU if we already know about it
	/// 1.1. Remove unsigned field
	/// 2. Check signatures, otherwise drop
	/// 3. Check content hash, redact if doesn't match
	/// 4. Fetch any missing auth events doing all checks listed here starting
	///    at 1. These are not timeline events
	/// 5. Reject "due to auth events" if can't get all the auth events or some
	///    of the auth events are also rejected "due to auth events"
	/// 6. Reject "due to auth events" if the event doesn't pass auth based on
	///    the auth events
	/// 7. Persist this event as an outlier
	/// 8. If not timeline event: stop
	/// 9. Fetch any missing prev events doing all checks listed here starting
	///    at 1. These are timeline events
	/// 10. Fetch missing state and auth chain events by calling `/state_ids` at
	///     backwards extremities doing all the checks in this list starting at
	///     1. These are not timeline events
	/// 11. Check the auth of the event passes based on the state of the event
	/// 12. Ensure that the state is derived from the previous current state
	///     (i.e. we calculated by doing state res where one of the inputs was a
	///     previously trusted set of state, don't just trust a set of state we
	///     got from a remote)
	/// 13. Use state resolution to find new room state
	/// 14. Check if the event passes auth based on the "current state" of the
	///     room, if not soft fail it
	#[tracing::instrument(skip(self, origin, value, is_timeline_event, pub_key_map), name = "pdu")]
	pub(crate) async fn handle_incoming_pdu<'a>(
		&self, origin: &'a ServerName, event_id: &'a EventId, room_id: &'a RoomId,
		value: BTreeMap<String, CanonicalJsonValue>, is_timeline_event: bool,
		pub_key_map: &'a RwLock<BTreeMap<String, BTreeMap<String, Base64>>>,
	) -> Result<Option<Vec<u8>>> {
		// 1. Skip the PDU if we already have it as a timeline event
		if let Some(pdu_id) = services().rooms.timeline.get_pdu_id(event_id)? {
			return Ok(Some(pdu_id));
		}

		// 1.1 Check the server is in the room
		if !services().rooms.metadata.exists(room_id)? {
			return Err(Error::BadRequest(ErrorKind::NotFound, "Room is unknown to this server"));
		}

		// 1.2 Check if the room is disabled
		if services().rooms.metadata.is_disabled(room_id)? {
			return Err(Error::BadRequest(
				ErrorKind::forbidden(),
				"Federation of this room is currently disabled on this server.",
			));
		}

		// 1.3 Check room ACL
		services().rooms.event_handler.acl_check(origin, room_id)?;

		// Fetch create event
		let create_event = services()
			.rooms
			.state_accessor
			.room_state_get(room_id, &StateEventType::RoomCreate, "")?
			.ok_or_else(|| Error::bad_database("Failed to find create event in db."))?;

		// Procure the room version
		let room_version_id = self.get_room_version_id(&create_event)?;

		let first_pdu_in_room = services()
			.rooms
			.timeline
			.first_pdu_in_room(room_id)?
			.ok_or_else(|| Error::bad_database("Failed to find first pdu in db."))?;

		let (incoming_pdu, val) = self
			.handle_outlier_pdu(origin, &create_event, event_id, room_id, value, false, pub_key_map)
			.await?;

		self.check_room_id(room_id, &incoming_pdu)?;

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
				pub_key_map,
				incoming_pdu.prev_events.clone(),
			)
			.await?;

		debug!(events = ?sorted_prev_events, "Got previous events");
		for prev_id in sorted_prev_events {
			match self
				.handle_prev_pdu(
					origin,
					event_id,
					room_id,
					pub_key_map,
					&mut eventid_info,
					&create_event,
					&first_pdu_in_room,
					&prev_id,
				)
				.await
			{
				Ok(()) => continue,
				Err(e) => {
					warn!("Prev event {} failed: {}", prev_id, e);
					match services()
						.globals
						.bad_event_ratelimiter
						.write()
						.await
						.entry((*prev_id).to_owned())
					{
						hash_map::Entry::Vacant(e) => {
							e.insert((Instant::now(), 1));
						},
						hash_map::Entry::Occupied(mut e) => {
							*e.get_mut() = (Instant::now(), e.get().1 + 1);
						},
					};
				},
			}
		}

		// Done with prev events, now handling the incoming event
		let start_time = Instant::now();
		services()
			.globals
			.roomid_federationhandletime
			.write()
			.await
			.insert(room_id.to_owned(), (event_id.to_owned(), start_time));

		let r = services()
			.rooms
			.event_handler
			.upgrade_outlier_to_timeline_pdu(incoming_pdu, val, &create_event, origin, room_id, pub_key_map)
			.await;

		services()
			.globals
			.roomid_federationhandletime
			.write()
			.await
			.remove(&room_id.to_owned());

		r
	}

	#[allow(clippy::type_complexity)]
	#[allow(clippy::too_many_arguments)]
	#[tracing::instrument(
		skip(self, origin, event_id, room_id, pub_key_map, eventid_info, create_event, first_pdu_in_room),
		name = "prev"
	)]
	pub(crate) async fn handle_prev_pdu<'a>(
		&self, origin: &'a ServerName, event_id: &'a EventId, room_id: &'a RoomId,
		pub_key_map: &'a RwLock<BTreeMap<String, BTreeMap<String, Base64>>>,
		eventid_info: &mut HashMap<Arc<EventId>, (Arc<PduEvent>, BTreeMap<String, CanonicalJsonValue>)>,
		create_event: &Arc<PduEvent>, first_pdu_in_room: &Arc<PduEvent>, prev_id: &EventId,
	) -> Result<()> {
		// Check for disabled again because it might have changed
		if services().rooms.metadata.is_disabled(room_id)? {
			debug!(
				"Federaton of room {room_id} is currently disabled on this server. Request by origin {origin} and \
				 event ID {event_id}"
			);
			return Err(Error::BadRequest(
				ErrorKind::forbidden(),
				"Federation of this room is currently disabled on this server.",
			));
		}

		if let Some((time, tries)) = services()
			.globals
			.bad_event_ratelimiter
			.read()
			.await
			.get(prev_id)
		{
			// Exponential backoff
			const MAX_DURATION: Duration = Duration::from_secs(60 * 60 * 24);
			let min_duration = cmp::min(MAX_DURATION, Duration::from_secs(5 * 60) * (*tries) * (*tries));
			let duration = time.elapsed();
			if duration < min_duration {
				debug!(
					duration = ?duration,
					min_duration = ?min_duration,
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
			services()
				.globals
				.roomid_federationhandletime
				.write()
				.await
				.insert(room_id.to_owned(), ((*prev_id).to_owned(), start_time));

			self.upgrade_outlier_to_timeline_pdu(pdu, json, create_event, origin, room_id, pub_key_map)
				.await?;

			services()
				.globals
				.roomid_federationhandletime
				.write()
				.await
				.remove(&room_id.to_owned());

			debug!(
				elapsed = ?start_time.elapsed(),
				"Handled prev_event",
			);
		}

		Ok(())
	}

	#[allow(clippy::too_many_arguments)]
	fn handle_outlier_pdu<'a>(
		&'a self, origin: &'a ServerName, create_event: &'a PduEvent, event_id: &'a EventId, room_id: &'a RoomId,
		mut value: BTreeMap<String, CanonicalJsonValue>, auth_events_known: bool,
		pub_key_map: &'a RwLock<BTreeMap<String, BTreeMap<String, Base64>>>,
	) -> AsyncRecursiveCanonicalJsonResult<'a> {
		Box::pin(async move {
			// 1. Remove unsigned field
			value.remove("unsigned");

			// TODO: For RoomVersion6 we must check that Raw<..> is canonical do we anywhere?: https://matrix.org/docs/spec/rooms/v6#canonical-json

			// 2. Check signatures, otherwise drop
			// 3. check content hash, redact if doesn't match
			let room_version_id = self.get_room_version_id(create_event)?;

			let guard = pub_key_map.read().await;
			let mut val = match ruma::signatures::verify_event(&guard, &value, &room_version_id) {
				Err(e) => {
					// Drop
					warn!("Dropping bad event {}: {}", event_id, e,);
					return Err(Error::BadRequest(ErrorKind::InvalidParam, "Signature verification failed"));
				},
				Ok(ruma::signatures::Verified::Signatures) => {
					// Redact
					warn!("Calculated hash does not match: {}", event_id);
					let Ok(obj) = ruma::canonical_json::redact(value, &room_version_id, None) else {
						return Err(Error::BadRequest(ErrorKind::InvalidParam, "Redaction failed"));
					};

					// Skip the PDU if it is redacted and we already have it as an outlier event
					if services().rooms.timeline.get_pdu_json(event_id)?.is_some() {
						return Err(Error::BadRequest(
							ErrorKind::InvalidParam,
							"Event was redacted and we already knew about it",
						));
					}

					obj
				},
				Ok(ruma::signatures::Verified::All) => value,
			};

			drop(guard);

			// Now that we have checked the signature and hashes we can add the eventID and
			// convert to our PduEvent type
			val.insert("event_id".to_owned(), CanonicalJsonValue::String(event_id.as_str().to_owned()));
			let incoming_pdu = serde_json::from_value::<PduEvent>(
				serde_json::to_value(&val).expect("CanonicalJsonObj is a valid JsonValue"),
			)
			.map_err(|_| Error::bad_database("Event is not a valid PDU."))?;

			self.check_room_id(room_id, &incoming_pdu)?;

			if !auth_events_known {
				// 4. fetch any missing auth events doing all checks listed here starting at 1.
				//    These are not timeline events
				// 5. Reject "due to auth events" if can't get all the auth events or some of
				//    the auth events are also rejected "due to auth events"
				// NOTE: Step 5 is not applied anymore because it failed too often
				debug!("Fetching auth events");
				self.fetch_and_handle_outliers(
					origin,
					&incoming_pdu
						.auth_events
						.iter()
						.map(|x| Arc::from(&**x))
						.collect::<Vec<_>>(),
					create_event,
					room_id,
					&room_version_id,
					pub_key_map,
				)
				.await;
			}

			// 6. Reject "due to auth events" if the event doesn't pass auth based on the
			//    auth events
			debug!("Checking based on auth events");
			// Build map of auth events
			let mut auth_events = HashMap::new();
			for id in &incoming_pdu.auth_events {
				let Some(auth_event) = services().rooms.timeline.get_pdu(id)? else {
					warn!("Could not find auth event {}", id);
					continue;
				};

				self.check_room_id(room_id, &auth_event)?;

				match auth_events.entry((
					auth_event.kind.to_string().into(),
					auth_event
						.state_key
						.clone()
						.expect("all auth events have state keys"),
				)) {
					hash_map::Entry::Vacant(v) => {
						v.insert(auth_event);
					},
					hash_map::Entry::Occupied(_) => {
						return Err(Error::BadRequest(
							ErrorKind::InvalidParam,
							"Auth event's type and state_key combination exists multiple times.",
						));
					},
				}
			}

			// The original create event must be in the auth events
			if !matches!(
				auth_events
					.get(&(StateEventType::RoomCreate, String::new()))
					.map(AsRef::as_ref),
				Some(_) | None
			) {
				return Err(Error::BadRequest(
					ErrorKind::InvalidParam,
					"Incoming event refers to wrong create event.",
				));
			}

			if !state_res::event_auth::auth_check(
				&self.to_room_version(&room_version_id),
				&incoming_pdu,
				None::<PduEvent>, // TODO: third party invite
				|k, s| auth_events.get(&(k.to_string().into(), s.to_owned())),
			)
			.map_err(|_e| Error::BadRequest(ErrorKind::InvalidParam, "Auth check failed"))?
			{
				return Err(Error::BadRequest(ErrorKind::InvalidParam, "Auth check failed"));
			}

			debug!("Validation successful.");

			// 7. Persist the event as an outlier.
			services()
				.rooms
				.outlier
				.add_pdu_outlier(&incoming_pdu.event_id, &val)?;

			debug!("Added pdu as outlier.");

			Ok((Arc::new(incoming_pdu), val))
		})
	}

	pub async fn upgrade_outlier_to_timeline_pdu(
		&self, incoming_pdu: Arc<PduEvent>, val: BTreeMap<String, CanonicalJsonValue>, create_event: &PduEvent,
		origin: &ServerName, room_id: &RoomId, pub_key_map: &RwLock<BTreeMap<String, BTreeMap<String, Base64>>>,
	) -> Result<Option<Vec<u8>>> {
		// Skip the PDU if we already have it as a timeline event
		if let Ok(Some(pduid)) = services().rooms.timeline.get_pdu_id(&incoming_pdu.event_id) {
			return Ok(Some(pduid));
		}

		if services()
			.rooms
			.pdu_metadata
			.is_event_soft_failed(&incoming_pdu.event_id)?
		{
			return Err(Error::BadRequest(ErrorKind::InvalidParam, "Event has been soft failed"));
		}

		debug!("Upgrading to timeline pdu");
		let timer = tokio::time::Instant::now();
		let room_version_id = self.get_room_version_id(create_event)?;

		// 10. Fetch missing state and auth chain events by calling /state_ids at
		//     backwards extremities doing all the checks in this list starting at 1.
		//     These are not timeline events.

		debug!("Resolving state at event");
		let mut state_at_incoming_event = if incoming_pdu.prev_events.len() == 1 {
			self.state_at_incoming_degree_one(&incoming_pdu).await?
		} else {
			self.state_at_incoming_resolved(&incoming_pdu, room_id, &room_version_id)
				.await?
		};

		if state_at_incoming_event.is_none() {
			state_at_incoming_event = self
				.fetch_state(
					origin,
					create_event,
					room_id,
					&room_version_id,
					pub_key_map,
					&incoming_pdu.event_id,
				)
				.await?;
		}

		let state_at_incoming_event = state_at_incoming_event.expect("we always set this to some above");
		let room_version = self.to_room_version(&room_version_id);

		debug!("Performing auth check");
		// 11. Check the auth of the event passes based on the state of the event
		let check_result = state_res::event_auth::auth_check(
			&room_version,
			&incoming_pdu,
			None::<PduEvent>, // TODO: third party invite
			|k, s| {
				services()
					.rooms
					.short
					.get_shortstatekey(&k.to_string().into(), s)
					.ok()
					.flatten()
					.and_then(|shortstatekey| state_at_incoming_event.get(&shortstatekey))
					.and_then(|event_id| services().rooms.timeline.get_pdu(event_id).ok().flatten())
			},
		)
		.map_err(|_e| Error::BadRequest(ErrorKind::InvalidParam, "Auth check failed."))?;

		if !check_result {
			return Err(Error::bad_database("Event has failed auth check with state at the event."));
		}

		debug!("Gathering auth events");
		let auth_events = services().rooms.state.get_auth_events(
			room_id,
			&incoming_pdu.kind,
			&incoming_pdu.sender,
			incoming_pdu.state_key.as_deref(),
			&incoming_pdu.content,
		)?;

		// Soft fail check before doing state res
		debug!("Performing soft-fail check");
		let soft_fail = !state_res::event_auth::auth_check(&room_version, &incoming_pdu, None::<PduEvent>, |k, s| {
			auth_events.get(&(k.clone(), s.to_owned()))
		})
		.map_err(|_e| Error::BadRequest(ErrorKind::InvalidParam, "Auth check failed."))?;

		// 13. Use state resolution to find new room state

		// We start looking at current room state now, so lets lock the room
		let mutex_state = Arc::clone(
			services()
				.globals
				.roomid_mutex_state
				.write()
				.await
				.entry(room_id.to_owned())
				.or_default(),
		);

		debug!("Locking the room");
		let state_lock = mutex_state.lock().await;

		// Now we calculate the set of extremities this room has after the incoming
		// event has been applied. We start with the previous extremities (aka leaves)
		debug!("Calculating extremities");
		let mut extremities = services().rooms.state.get_forward_extremities(room_id)?;
		debug!("Calculated {} extremities", extremities.len());

		// Remove any forward extremities that are referenced by this incoming event's
		// prev_events
		for prev_event in &incoming_pdu.prev_events {
			extremities.remove(prev_event);
		}

		// Only keep those extremities were not referenced yet
		extremities.retain(|id| {
			!matches!(
				services()
					.rooms
					.pdu_metadata
					.is_event_referenced(room_id, id),
				Ok(true)
			)
		});
		debug!("Retained {} extremities. Compressing state", extremities.len());
		let state_ids_compressed = Arc::new(
			state_at_incoming_event
				.iter()
				.map(|(shortstatekey, id)| {
					services()
						.rooms
						.state_compressor
						.compress_state_event(*shortstatekey, id)
				})
				.collect::<Result<_>>()?,
		);

		if incoming_pdu.state_key.is_some() {
			debug!("Event is a state-event. Deriving new room state");

			// We also add state after incoming event to the fork states
			let mut state_after = state_at_incoming_event.clone();
			if let Some(state_key) = &incoming_pdu.state_key {
				let shortstatekey = services()
					.rooms
					.short
					.get_or_create_shortstatekey(&incoming_pdu.kind.to_string().into(), state_key)?;

				state_after.insert(shortstatekey, Arc::from(&*incoming_pdu.event_id));
			}

			let new_room_state = self
				.resolve_state(room_id, &room_version_id, state_after)
				.await?;

			// Set the new room state to the resolved state
			debug!("Forcing new room state");
			let (sstatehash, new, removed) = services()
				.rooms
				.state_compressor
				.save_state(room_id, new_room_state)?;

			services()
				.rooms
				.state
				.force_state(room_id, sstatehash, new, removed, &state_lock)
				.await?;
		}

		// 14. Check if the event passes auth based on the "current state" of the room,
		//     if not soft fail it
		if soft_fail {
			debug!("Soft failing event");
			services()
				.rooms
				.timeline
				.append_incoming_pdu(
					&incoming_pdu,
					val,
					extremities.iter().map(|e| (**e).to_owned()).collect(),
					state_ids_compressed,
					soft_fail,
					&state_lock,
				)
				.await?;

			// Soft fail, we keep the event as an outlier but don't add it to the timeline
			warn!("Event was soft failed: {:?}", incoming_pdu);
			services()
				.rooms
				.pdu_metadata
				.mark_event_soft_failed(&incoming_pdu.event_id)?;

			return Err(Error::BadRequest(ErrorKind::InvalidParam, "Event has been soft failed"));
		}

		debug!("Appending pdu to timeline");
		extremities.insert(incoming_pdu.event_id.clone());

		// Now that the event has passed all auth it is added into the timeline.
		// We use the `state_at_event` instead of `state_after` so we accurately
		// represent the state for this event.
		let pdu_id = services()
			.rooms
			.timeline
			.append_incoming_pdu(
				&incoming_pdu,
				val,
				extremities.iter().map(|e| (**e).to_owned()).collect(),
				state_ids_compressed,
				soft_fail,
				&state_lock,
			)
			.await?;

		// Event has passed all auth/stateres checks
		drop(state_lock);
		debug!(
			elapsed = ?timer.elapsed(),
			"Appended incoming pdu",
		);

		Ok(pdu_id)
	}

	async fn resolve_state(
		&self, room_id: &RoomId, room_version_id: &RoomVersionId, incoming_state: HashMap<u64, Arc<EventId>>,
	) -> Result<Arc<HashSet<CompressedStateEvent>>> {
		debug!("Loading current room state ids");
		let current_sstatehash = services()
			.rooms
			.state
			.get_room_shortstatehash(room_id)?
			.expect("every room has state");

		let current_state_ids = services()
			.rooms
			.state_accessor
			.state_full_ids(current_sstatehash)
			.await?;

		let fork_states = [current_state_ids, incoming_state];

		let mut auth_chain_sets = Vec::new();
		for state in &fork_states {
			auth_chain_sets.push(
				services()
					.rooms
					.auth_chain
					.event_ids_iter(room_id, state.iter().map(|(_, id)| id.clone()).collect())
					.await?
					.collect(),
			);
		}

		debug!("Loading fork states");
		let fork_states: Vec<_> = fork_states
			.into_iter()
			.map(|map| {
				map.into_iter()
					.filter_map(|(k, id)| {
						services()
							.rooms
							.short
							.get_statekey_from_short(k)
							.map(|(ty, st_key)| ((ty.to_string().into(), st_key), id))
							.ok()
					})
					.collect::<StateMap<_>>()
			})
			.collect();

		let lock = services().globals.stateres_mutex.lock();

		debug!("Resolving state");
		let state_resolve = state_res::resolve(room_version_id, &fork_states, auth_chain_sets, |id| {
			let res = services().rooms.timeline.get_pdu(id);
			if let Err(e) = &res {
				error!("Failed to fetch event: {}", e);
			}
			res.ok().flatten()
		});

		let state = match state_resolve {
			Ok(new_state) => new_state,
			Err(e) => {
				error!("State resolution failed: {}", e);
				return Err(Error::bad_database(
					"State resolution failed, either an event could not be found or deserialization",
				));
			},
		};

		drop(lock);

		debug!("State resolution done. Compressing state");
		let new_room_state = state
			.into_iter()
			.map(|((event_type, state_key), event_id)| {
				let shortstatekey = services()
					.rooms
					.short
					.get_or_create_shortstatekey(&event_type.to_string().into(), &state_key)?;
				services()
					.rooms
					.state_compressor
					.compress_state_event(shortstatekey, &event_id)
			})
			.collect::<Result<_>>()?;

		Ok(Arc::new(new_room_state))
	}

	// TODO: if we know the prev_events of the incoming event we can avoid the
	// request and build the state from a known point and resolve if > 1 prev_event
	#[tracing::instrument(skip_all, name = "state")]
	pub async fn state_at_incoming_degree_one(
		&self, incoming_pdu: &Arc<PduEvent>,
	) -> Result<Option<HashMap<u64, Arc<EventId>>>> {
		let prev_event = &*incoming_pdu.prev_events[0];
		let prev_event_sstatehash = services()
			.rooms
			.state_accessor
			.pdu_shortstatehash(prev_event)?;

		let state = if let Some(shortstatehash) = prev_event_sstatehash {
			Some(
				services()
					.rooms
					.state_accessor
					.state_full_ids(shortstatehash)
					.await,
			)
		} else {
			None
		};

		if let Some(Ok(mut state)) = state {
			debug!("Using cached state");
			let prev_pdu = services()
				.rooms
				.timeline
				.get_pdu(prev_event)
				.ok()
				.flatten()
				.ok_or_else(|| Error::bad_database("Could not find prev event, but we know the state."))?;

			if let Some(state_key) = &prev_pdu.state_key {
				let shortstatekey = services()
					.rooms
					.short
					.get_or_create_shortstatekey(&prev_pdu.kind.to_string().into(), state_key)?;

				state.insert(shortstatekey, Arc::from(prev_event));
				// Now it's the state after the pdu
			}

			return Ok(Some(state));
		}

		Ok(None)
	}

	#[tracing::instrument(skip_all, name = "state")]
	pub async fn state_at_incoming_resolved(
		&self, incoming_pdu: &Arc<PduEvent>, room_id: &RoomId, room_version_id: &RoomVersionId,
	) -> Result<Option<HashMap<u64, Arc<EventId>>>> {
		debug!("Calculating state at event using state res");
		let mut extremity_sstatehashes = HashMap::new();

		let mut okay = true;
		for prev_eventid in &incoming_pdu.prev_events {
			let Ok(Some(prev_event)) = services().rooms.timeline.get_pdu(prev_eventid) else {
				okay = false;
				break;
			};

			let Ok(Some(sstatehash)) = services()
				.rooms
				.state_accessor
				.pdu_shortstatehash(prev_eventid)
			else {
				okay = false;
				break;
			};

			extremity_sstatehashes.insert(sstatehash, prev_event);
		}

		if !okay {
			return Ok(None);
		}

		let mut fork_states = Vec::with_capacity(extremity_sstatehashes.len());
		let mut auth_chain_sets = Vec::with_capacity(extremity_sstatehashes.len());

		for (sstatehash, prev_event) in extremity_sstatehashes {
			let mut leaf_state: HashMap<_, _> = services()
				.rooms
				.state_accessor
				.state_full_ids(sstatehash)
				.await?;

			if let Some(state_key) = &prev_event.state_key {
				let shortstatekey = services()
					.rooms
					.short
					.get_or_create_shortstatekey(&prev_event.kind.to_string().into(), state_key)?;
				leaf_state.insert(shortstatekey, Arc::from(&*prev_event.event_id));
				// Now it's the state after the pdu
			}

			let mut state = StateMap::with_capacity(leaf_state.len());
			let mut starting_events = Vec::with_capacity(leaf_state.len());

			for (k, id) in leaf_state {
				if let Ok((ty, st_key)) = services().rooms.short.get_statekey_from_short(k) {
					// FIXME: Undo .to_string().into() when StateMap
					//        is updated to use StateEventType
					state.insert((ty.to_string().into(), st_key), id.clone());
				} else {
					warn!("Failed to get_statekey_from_short.");
				}
				starting_events.push(id);
			}

			auth_chain_sets.push(
				services()
					.rooms
					.auth_chain
					.event_ids_iter(room_id, starting_events)
					.await?
					.collect(),
			);

			fork_states.push(state);
		}

		let lock = services().globals.stateres_mutex.lock();
		let result = state_res::resolve(room_version_id, &fork_states, auth_chain_sets, |id| {
			let res = services().rooms.timeline.get_pdu(id);
			if let Err(e) = &res {
				error!("Failed to fetch event: {}", e);
			}
			res.ok().flatten()
		});
		drop(lock);

		Ok(match result {
			Ok(new_state) => Some(
				new_state
					.into_iter()
					.map(|((event_type, state_key), event_id)| {
						let shortstatekey = services()
							.rooms
							.short
							.get_or_create_shortstatekey(&event_type.to_string().into(), &state_key)?;
						Ok((shortstatekey, event_id))
					})
					.collect::<Result<_>>()?,
			),
			Err(e) => {
				warn!(
					"State resolution on prev events failed, either an event could not be found or deserialization: {}",
					e
				);
				None
			},
		})
	}

	/// Call /state_ids to find out what the state at this pdu is. We trust the
	/// server's response to some extend (sic), but we still do a lot of checks
	/// on the events
	#[tracing::instrument(skip_all)]
	async fn fetch_state(
		&self, origin: &ServerName, create_event: &PduEvent, room_id: &RoomId, room_version_id: &RoomVersionId,
		pub_key_map: &RwLock<BTreeMap<String, BTreeMap<String, Base64>>>, event_id: &EventId,
	) -> Result<Option<HashMap<u64, Arc<EventId>>>> {
		debug!("Fetching state ids");
		match services()
			.sending
			.send_federation_request(
				origin,
				get_room_state_ids::v1::Request {
					room_id: room_id.to_owned(),
					event_id: (*event_id).to_owned(),
				},
			)
			.await
		{
			Ok(res) => {
				debug!("Fetching state events");
				let collect = res
					.pdu_ids
					.iter()
					.map(|x| Arc::from(&**x))
					.collect::<Vec<_>>();

				let state_vec = self
					.fetch_and_handle_outliers(origin, &collect, create_event, room_id, room_version_id, pub_key_map)
					.await;

				let mut state: HashMap<_, Arc<EventId>> = HashMap::new();
				for (pdu, _) in state_vec {
					let state_key = pdu
						.state_key
						.clone()
						.ok_or_else(|| Error::bad_database("Found non-state pdu in state events."))?;

					let shortstatekey = services()
						.rooms
						.short
						.get_or_create_shortstatekey(&pdu.kind.to_string().into(), &state_key)?;

					match state.entry(shortstatekey) {
						hash_map::Entry::Vacant(v) => {
							v.insert(Arc::from(&*pdu.event_id));
						},
						hash_map::Entry::Occupied(_) => {
							return Err(Error::bad_database(
								"State event's type and state_key combination exists multiple times.",
							))
						},
					}
				}

				// The original create event must still be in the state
				let create_shortstatekey = services()
					.rooms
					.short
					.get_shortstatekey(&StateEventType::RoomCreate, "")?
					.expect("Room exists");

				if state.get(&create_shortstatekey).map(AsRef::as_ref) != Some(&create_event.event_id) {
					return Err(Error::bad_database("Incoming event refers to wrong create event."));
				}

				Ok(Some(state))
			},
			Err(e) => {
				warn!("Fetching state for event failed: {}", e);
				Err(e)
			},
		}
	}

	/// Find the event and auth it. Once the event is validated (steps 1 - 8)
	/// it is appended to the outliers Tree.
	///
	/// Returns pdu and if we fetched it over federation the raw json.
	///
	/// a. Look in the main timeline (pduid_pdu tree)
	/// b. Look at outlier pdu tree
	/// c. Ask origin server over federation
	/// d. TODO: Ask other servers over federation?
	pub(crate) fn fetch_and_handle_outliers<'a>(
		&'a self, origin: &'a ServerName, events: &'a [Arc<EventId>], create_event: &'a PduEvent, room_id: &'a RoomId,
		room_version_id: &'a RoomVersionId, pub_key_map: &'a RwLock<BTreeMap<String, BTreeMap<String, Base64>>>,
	) -> AsyncRecursiveCanonicalJsonVec<'a> {
		Box::pin(async move {
			let back_off = |id| async {
				match services()
					.globals
					.bad_event_ratelimiter
					.write()
					.await
					.entry(id)
				{
					hash_map::Entry::Vacant(e) => {
						e.insert((Instant::now(), 1));
					},
					hash_map::Entry::Occupied(mut e) => *e.get_mut() = (Instant::now(), e.get().1 + 1),
				}
			};

			let mut events_with_auth_events = vec![];
			for id in events {
				// a. Look in the main timeline (pduid_pdu tree)
				// b. Look at outlier pdu tree
				// (get_pdu_json checks both)
				if let Ok(Some(local_pdu)) = services().rooms.timeline.get_pdu(id) {
					trace!("Found {} in db", id);
					events_with_auth_events.push((id, Some(local_pdu), vec![]));
					continue;
				}

				// c. Ask origin server over federation
				// We also handle its auth chain here so we don't get a stack overflow in
				// handle_outlier_pdu.
				let mut todo_auth_events = vec![Arc::clone(id)];
				let mut events_in_reverse_order = Vec::new();
				let mut events_all = HashSet::new();
				let mut i = 0;
				while let Some(next_id) = todo_auth_events.pop() {
					if let Some((time, tries)) = services()
						.globals
						.bad_event_ratelimiter
						.read()
						.await
						.get(&*next_id)
					{
						// Exponential backoff
						let mut min_elapsed_duration = Duration::from_secs(5 * 60) * (*tries) * (*tries);
						if min_elapsed_duration > Duration::from_secs(60 * 60 * 24) {
							min_elapsed_duration = Duration::from_secs(60 * 60 * 24);
						}

						if time.elapsed() < min_elapsed_duration {
							info!("Backing off from {}", next_id);
							continue;
						}
					}

					if events_all.contains(&next_id) {
						continue;
					}

					i += 1;
					if i % 100 == 0 {
						tokio::task::yield_now().await;
					}

					if let Ok(Some(_)) = services().rooms.timeline.get_pdu(&next_id) {
						trace!("Found {} in db", next_id);
						continue;
					}

					debug!("Fetching {} over federation.", next_id);
					match services()
						.sending
						.send_federation_request(
							origin,
							get_event::v1::Request {
								event_id: (*next_id).to_owned(),
							},
						)
						.await
					{
						Ok(res) => {
							debug!("Got {} over federation", next_id);
							let Ok((calculated_event_id, value)) =
								pdu::gen_event_id_canonical_json(&res.pdu, room_version_id)
							else {
								back_off((*next_id).to_owned()).await;
								continue;
							};

							if calculated_event_id != *next_id {
								warn!(
									"Server didn't return event id we requested: requested: {}, we got {}. Event: {:?}",
									next_id, calculated_event_id, &res.pdu
								);
							}

							if let Some(auth_events) = value.get("auth_events").and_then(|c| c.as_array()) {
								for auth_event in auth_events {
									if let Ok(auth_event) = serde_json::from_value(auth_event.clone().into()) {
										let a: Arc<EventId> = auth_event;
										todo_auth_events.push(a);
									} else {
										warn!("Auth event id is not valid");
									}
								}
							} else {
								warn!("Auth event list invalid");
							}

							events_in_reverse_order.push((next_id.clone(), value));
							events_all.insert(next_id);
						},
						Err(e) => {
							warn!("Failed to fetch event {next_id}: {e}");
							back_off((*next_id).to_owned()).await;
						},
					}
				}
				events_with_auth_events.push((id, None, events_in_reverse_order));
			}

			// We go through all the signatures we see on the PDUs and their unresolved
			// dependencies and fetch the corresponding signing keys
			self.fetch_required_signing_keys(
				events_with_auth_events
					.iter()
					.flat_map(|(_id, _local_pdu, events)| events)
					.map(|(_event_id, event)| event),
				pub_key_map,
			)
			.await
			.unwrap_or_else(|e| {
				warn!("Could not fetch all signatures for PDUs from {}: {:?}", origin, e);
			});

			let mut pdus = vec![];
			for (id, local_pdu, events_in_reverse_order) in events_with_auth_events {
				// a. Look in the main timeline (pduid_pdu tree)
				// b. Look at outlier pdu tree
				// (get_pdu_json checks both)
				if let Some(local_pdu) = local_pdu {
					trace!("Found {} in db", id);
					pdus.push((local_pdu, None));
				}
				for (next_id, value) in events_in_reverse_order.iter().rev() {
					if let Some((time, tries)) = services()
						.globals
						.bad_event_ratelimiter
						.read()
						.await
						.get(&**next_id)
					{
						// Exponential backoff
						let mut min_elapsed_duration = Duration::from_secs(5 * 60) * (*tries) * (*tries);
						if min_elapsed_duration > Duration::from_secs(60 * 60 * 24) {
							min_elapsed_duration = Duration::from_secs(60 * 60 * 24);
						}

						if time.elapsed() < min_elapsed_duration {
							debug!("Backing off from {}", next_id);
							continue;
						}
					}

					match self
						.handle_outlier_pdu(origin, create_event, next_id, room_id, value.clone(), true, pub_key_map)
						.await
					{
						Ok((pdu, json)) => {
							if next_id == id {
								pdus.push((pdu, Some(json)));
							}
						},
						Err(e) => {
							warn!("Authentication of event {} failed: {:?}", next_id, e);
							back_off((**next_id).to_owned()).await;
						},
					}
				}
			}
			pdus
		})
	}

	#[allow(clippy::type_complexity)]
	#[tracing::instrument(skip_all)]
	async fn fetch_prev(
		&self, origin: &ServerName, create_event: &PduEvent, room_id: &RoomId, room_version_id: &RoomVersionId,
		pub_key_map: &RwLock<BTreeMap<String, BTreeMap<String, Base64>>>, initial_set: Vec<Arc<EventId>>,
	) -> Result<(
		Vec<Arc<EventId>>,
		HashMap<Arc<EventId>, (Arc<PduEvent>, BTreeMap<String, CanonicalJsonValue>)>,
	)> {
		let mut graph: HashMap<Arc<EventId>, _> = HashMap::new();
		let mut eventid_info = HashMap::new();
		let mut todo_outlier_stack: Vec<Arc<EventId>> = initial_set;

		let first_pdu_in_room = services()
			.rooms
			.timeline
			.first_pdu_in_room(room_id)?
			.ok_or_else(|| Error::bad_database("Failed to find first pdu in db."))?;

		let mut amount = 0;

		while let Some(prev_event_id) = todo_outlier_stack.pop() {
			if let Some((pdu, json_opt)) = self
				.fetch_and_handle_outliers(
					origin,
					&[prev_event_id.clone()],
					create_event,
					room_id,
					room_version_id,
					pub_key_map,
				)
				.await
				.pop()
			{
				self.check_room_id(room_id, &pdu)?;

				if amount > services().globals.max_fetch_prev_events() {
					// Max limit reached
					debug!(
						"Max prev event limit reached! Limit: {}",
						services().globals.max_fetch_prev_events()
					);
					graph.insert(prev_event_id.clone(), HashSet::new());
					continue;
				}

				if let Some(json) = json_opt.or_else(|| {
					services()
						.rooms
						.outlier
						.get_outlier_pdu_json(&prev_event_id)
						.ok()
						.flatten()
				}) {
					if pdu.origin_server_ts > first_pdu_in_room.origin_server_ts {
						amount += 1;
						for prev_prev in &pdu.prev_events {
							if !graph.contains_key(prev_prev) {
								todo_outlier_stack.push(prev_prev.clone());
							}
						}

						graph.insert(prev_event_id.clone(), pdu.prev_events.iter().cloned().collect());
					} else {
						// Time based check failed
						graph.insert(prev_event_id.clone(), HashSet::new());
					}

					eventid_info.insert(prev_event_id.clone(), (pdu, json));
				} else {
					// Get json failed, so this was not fetched over federation
					graph.insert(prev_event_id.clone(), HashSet::new());
				}
			} else {
				// Fetch and handle failed
				graph.insert(prev_event_id.clone(), HashSet::new());
			}
		}

		let sorted = state_res::lexicographical_topological_sort(&graph, |event_id| {
			// This return value is the key used for sorting events,
			// events are then sorted by power level, time,
			// and lexically by event_id.
			Ok((
				int!(0),
				MilliSecondsSinceUnixEpoch(
					eventid_info
						.get(event_id)
						.map_or_else(|| uint!(0), |info| info.0.origin_server_ts),
				),
			))
		})
		.map_err(|e| {
			error!("Error sorting prev events: {e}");
			Error::bad_database("Error sorting prev events")
		})?;

		Ok((sorted, eventid_info))
	}

	/// Returns Ok if the acl allows the server
	#[tracing::instrument(skip_all)]
	pub fn acl_check(&self, server_name: &ServerName, room_id: &RoomId) -> Result<()> {
		let acl_event = if let Some(acl) =
			services()
				.rooms
				.state_accessor
				.room_state_get(room_id, &StateEventType::RoomServerAcl, "")?
		{
			trace!("ACL event found: {acl:?}");
			acl
		} else {
			trace!("No ACL event found");
			return Ok(());
		};

		let acl_event_content: RoomServerAclEventContent = match serde_json::from_str(acl_event.content.get()) {
			Ok(content) => {
				trace!("Found ACL event contents: {content:?}");
				content
			},
			Err(e) => {
				warn!("Invalid ACL event: {e}");
				return Ok(());
			},
		};

		if acl_event_content.allow.is_empty() {
			warn!("Ignoring broken ACL event (allow key is empty)");
			// Ignore broken acl events
			return Ok(());
		}

		if acl_event_content.is_allowed(server_name) {
			trace!("server {server_name} is allowed by ACL");
			Ok(())
		} else {
			debug!("Server {} was denied by room ACL in {}", server_name, room_id);
			Err(Error::BadRequest(ErrorKind::forbidden(), "Server was denied by room ACL"))
		}
	}

	fn check_room_id(&self, room_id: &RoomId, pdu: &PduEvent) -> Result<()> {
		if pdu.room_id != room_id {
			warn!("Found event from room {} in room {}", pdu.room_id, room_id);
			return Err(Error::BadRequest(ErrorKind::InvalidParam, "Event has wrong room id"));
		}
		Ok(())
	}

	fn get_room_version_id(&self, create_event: &PduEvent) -> Result<RoomVersionId> {
		let create_event_content: RoomCreateEventContent =
			serde_json::from_str(create_event.content.get()).map_err(|e| {
				error!("Invalid create event: {}", e);
				Error::BadDatabase("Invalid create event in db")
			})?;

		Ok(create_event_content.room_version)
	}

	fn to_room_version(&self, room_version_id: &RoomVersionId) -> RoomVersion {
		RoomVersion::new(room_version_id).expect("room version is supported")
	}
}
