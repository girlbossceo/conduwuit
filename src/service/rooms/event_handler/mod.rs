mod parse_incoming_pdu;

use std::{
	borrow::Borrow,
	collections::{hash_map, BTreeMap, HashMap, HashSet},
	fmt::Write,
	sync::{Arc, RwLock as StdRwLock},
	time::Instant,
};

use conduit::{
	debug, debug_error, debug_info, debug_warn, err, info, pdu,
	result::LogErr,
	trace,
	utils::{math::continue_exponential_backoff_secs, IterStream, MutexMap},
	warn, Err, Error, PduEvent, Result,
};
use futures::{future, future::ready, FutureExt, StreamExt, TryFutureExt};
use ruma::{
	api::{
		client::error::ErrorKind,
		federation::event::{get_event, get_room_state_ids},
	},
	events::{
		room::{
			create::RoomCreateEventContent, redaction::RoomRedactionEventContent, server_acl::RoomServerAclEventContent,
		},
		StateEventType, TimelineEventType,
	},
	int,
	serde::Base64,
	state_res::{self, EventTypeExt, RoomVersion, StateMap},
	uint, CanonicalJsonValue, EventId, MilliSecondsSinceUnixEpoch, OwnedEventId, OwnedRoomId, OwnedUserId, RoomId,
	RoomVersionId, ServerName,
};
use tokio::sync::RwLock;

use super::state_compressor::CompressedStateEvent;
use crate::{globals, rooms, sending, server_keys, Dep};

pub struct Service {
	services: Services,
	pub federation_handletime: StdRwLock<HandleTimeMap>,
	pub mutex_federation: RoomMutexMap,
}

struct Services {
	globals: Dep<globals::Service>,
	sending: Dep<sending::Service>,
	auth_chain: Dep<rooms::auth_chain::Service>,
	metadata: Dep<rooms::metadata::Service>,
	outlier: Dep<rooms::outlier::Service>,
	pdu_metadata: Dep<rooms::pdu_metadata::Service>,
	server_keys: Dep<server_keys::Service>,
	short: Dep<rooms::short::Service>,
	state: Dep<rooms::state::Service>,
	state_accessor: Dep<rooms::state_accessor::Service>,
	state_compressor: Dep<rooms::state_compressor::Service>,
	timeline: Dep<rooms::timeline::Service>,
}

type RoomMutexMap = MutexMap<OwnedRoomId, ()>;
type HandleTimeMap = HashMap<OwnedRoomId, (OwnedEventId, Instant)>;

impl crate::Service for Service {
	fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
		Ok(Arc::new(Self {
			services: Services {
				globals: args.depend::<globals::Service>("globals"),
				sending: args.depend::<sending::Service>("sending"),
				auth_chain: args.depend::<rooms::auth_chain::Service>("rooms::auth_chain"),
				metadata: args.depend::<rooms::metadata::Service>("rooms::metadata"),
				outlier: args.depend::<rooms::outlier::Service>("rooms::outlier"),
				server_keys: args.depend::<server_keys::Service>("server_keys"),
				pdu_metadata: args.depend::<rooms::pdu_metadata::Service>("rooms::pdu_metadata"),
				short: args.depend::<rooms::short::Service>("rooms::short"),
				state: args.depend::<rooms::state::Service>("rooms::state"),
				state_accessor: args.depend::<rooms::state_accessor::Service>("rooms::state_accessor"),
				state_compressor: args.depend::<rooms::state_compressor::Service>("rooms::state_compressor"),
				timeline: args.depend::<rooms::timeline::Service>("rooms::timeline"),
			},
			federation_handletime: HandleTimeMap::new().into(),
			mutex_federation: RoomMutexMap::new(),
		}))
	}

	fn memory_usage(&self, out: &mut dyn Write) -> Result<()> {
		let mutex_federation = self.mutex_federation.len();
		writeln!(out, "federation_mutex: {mutex_federation}")?;

		let federation_handletime = self
			.federation_handletime
			.read()
			.expect("locked for reading")
			.len();
		writeln!(out, "federation_handletime: {federation_handletime}")?;

		Ok(())
	}

	fn name(&self) -> &str { crate::service::make_name(std::module_path!()) }
}

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
	pub async fn handle_incoming_pdu<'a>(
		&self, origin: &'a ServerName, room_id: &'a RoomId, event_id: &'a EventId,
		value: BTreeMap<String, CanonicalJsonValue>, is_timeline_event: bool,
		pub_key_map: &'a RwLock<BTreeMap<String, BTreeMap<String, Base64>>>,
	) -> Result<Option<Vec<u8>>> {
		// 1. Skip the PDU if we already have it as a timeline event
		if let Ok(pdu_id) = self.services.timeline.get_pdu_id(event_id).await {
			return Ok(Some(pdu_id.to_vec()));
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
		let sender: OwnedUserId = serde_json::from_value(
			value
				.get("sender")
				.ok_or_else(|| Error::BadRequest(ErrorKind::InvalidParam, "PDU does not have a sender key"))?
				.clone()
				.into(),
		)
		.map_err(|_| Error::BadRequest(ErrorKind::BadJson, "User ID in sender is invalid"))?;

		self.acl_check(sender.server_name(), room_id).await?;

		// Fetch create event
		let create_event = self
			.services
			.state_accessor
			.room_state_get(room_id, &StateEventType::RoomCreate, "")
			.await?;

		// Procure the room version
		let room_version_id = Self::get_room_version_id(&create_event)?;

		let first_pdu_in_room = self.services.timeline.first_pdu_in_room(room_id).await?;

		let (incoming_pdu, val) = self
			.handle_outlier_pdu(origin, &create_event, event_id, room_id, value, false, pub_key_map)
			.boxed()
			.await?;

		Self::check_room_id(room_id, &incoming_pdu)?;

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
					warn!("Prev event {prev_id} failed: {e}");
					match self
						.services
						.globals
						.bad_event_ratelimiter
						.write()
						.expect("locked")
						.entry((*prev_id).to_owned())
					{
						hash_map::Entry::Vacant(e) => {
							e.insert((Instant::now(), 1));
						},
						hash_map::Entry::Occupied(mut e) => {
							*e.get_mut() = (Instant::now(), e.get().1.saturating_add(1));
						},
					};
				},
			}
		}

		// Done with prev events, now handling the incoming event
		let start_time = Instant::now();
		self.federation_handletime
			.write()
			.expect("locked")
			.insert(room_id.to_owned(), (event_id.to_owned(), start_time));

		let r = self
			.upgrade_outlier_to_timeline_pdu(incoming_pdu, val, &create_event, origin, room_id, pub_key_map)
			.await;

		self.federation_handletime
			.write()
			.expect("locked")
			.remove(&room_id.to_owned());

		r
	}

	#[allow(clippy::type_complexity)]
	#[allow(clippy::too_many_arguments)]
	#[tracing::instrument(
		skip(self, origin, event_id, room_id, pub_key_map, eventid_info, create_event, first_pdu_in_room),
		name = "prev"
	)]
	pub async fn handle_prev_pdu<'a>(
		&self, origin: &'a ServerName, event_id: &'a EventId, room_id: &'a RoomId,
		pub_key_map: &'a RwLock<BTreeMap<String, BTreeMap<String, Base64>>>,
		eventid_info: &mut HashMap<Arc<EventId>, (Arc<PduEvent>, BTreeMap<String, CanonicalJsonValue>)>,
		create_event: &Arc<PduEvent>, first_pdu_in_room: &Arc<PduEvent>, prev_id: &EventId,
	) -> Result<()> {
		// Check for disabled again because it might have changed
		if self.services.metadata.is_disabled(room_id).await {
			debug!(
				"Federaton of room {room_id} is currently disabled on this server. Request by origin {origin} and \
				 event ID {event_id}"
			);
			return Err(Error::BadRequest(
				ErrorKind::forbidden(),
				"Federation of this room is currently disabled on this server.",
			));
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

			self.upgrade_outlier_to_timeline_pdu(pdu, json, create_event, origin, room_id, pub_key_map)
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

	#[allow(clippy::too_many_arguments)]
	async fn handle_outlier_pdu<'a>(
		&self, origin: &'a ServerName, create_event: &'a PduEvent, event_id: &'a EventId, room_id: &'a RoomId,
		mut value: BTreeMap<String, CanonicalJsonValue>, auth_events_known: bool,
		pub_key_map: &'a RwLock<BTreeMap<String, BTreeMap<String, Base64>>>,
	) -> Result<(Arc<PduEvent>, BTreeMap<String, CanonicalJsonValue>)> {
		// 1. Remove unsigned field
		value.remove("unsigned");

		// TODO: For RoomVersion6 we must check that Raw<..> is canonical do we anywhere?: https://matrix.org/docs/spec/rooms/v6#canonical-json

		// 2. Check signatures, otherwise drop
		// 3. check content hash, redact if doesn't match
		let room_version_id = Self::get_room_version_id(create_event)?;

		let guard = pub_key_map.read().await;
		let mut val = match ruma::signatures::verify_event(&guard, &value, &room_version_id) {
			Err(e) => {
				// Drop
				warn!("Dropping bad event {event_id}: {e}");
				return Err!(Request(InvalidParam("Signature verification failed")));
			},
			Ok(ruma::signatures::Verified::Signatures) => {
				// Redact
				debug_info!("Calculated hash does not match (redaction): {event_id}");
				let Ok(obj) = ruma::canonical_json::redact(value, &room_version_id, None) else {
					return Err!(Request(InvalidParam("Redaction failed")));
				};

				// Skip the PDU if it is redacted and we already have it as an outlier event
				if self.services.timeline.get_pdu_json(event_id).await.is_ok() {
					return Err!(Request(InvalidParam("Event was redacted and we already knew about it")));
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

		Self::check_room_id(room_id, &incoming_pdu)?;

		if !auth_events_known {
			// 4. fetch any missing auth events doing all checks listed here starting at 1.
			//    These are not timeline events
			// 5. Reject "due to auth events" if can't get all the auth events or some of
			//    the auth events are also rejected "due to auth events"
			// NOTE: Step 5 is not applied anymore because it failed too often
			debug!("Fetching auth events");
			Box::pin(
				self.fetch_and_handle_outliers(
					origin,
					&incoming_pdu
						.auth_events
						.iter()
						.map(|x| Arc::from(&**x))
						.collect::<Vec<Arc<EventId>>>(),
					create_event,
					room_id,
					&room_version_id,
					pub_key_map,
				),
			)
			.await;
		}

		// 6. Reject "due to auth events" if the event doesn't pass auth based on the
		//    auth events
		debug!("Checking based on auth events");
		// Build map of auth events
		let mut auth_events = HashMap::with_capacity(incoming_pdu.auth_events.len());
		for id in &incoming_pdu.auth_events {
			let Ok(auth_event) = self.services.timeline.get_pdu(id).await else {
				warn!("Could not find auth event {id}");
				continue;
			};

			Self::check_room_id(room_id, &auth_event)?;

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

		let state_fetch = |ty: &'static StateEventType, sk: &str| {
			let key = ty.with_state_key(sk);
			ready(auth_events.get(&key))
		};

		let auth_check = state_res::event_auth::auth_check(
			&Self::to_room_version(&room_version_id),
			&incoming_pdu,
			None, // TODO: third party invite
			state_fetch,
		)
		.await
		.map_err(|e| err!(Request(Forbidden("Auth check failed: {e:?}"))))?;

		if !auth_check {
			return Err!(Request(Forbidden("Auth check failed")));
		}

		trace!("Validation successful.");

		// 7. Persist the event as an outlier.
		self.services
			.outlier
			.add_pdu_outlier(&incoming_pdu.event_id, &val);

		trace!("Added pdu as outlier.");

		Ok((Arc::new(incoming_pdu), val))
	}

	pub async fn upgrade_outlier_to_timeline_pdu(
		&self, incoming_pdu: Arc<PduEvent>, val: BTreeMap<String, CanonicalJsonValue>, create_event: &PduEvent,
		origin: &ServerName, room_id: &RoomId, pub_key_map: &RwLock<BTreeMap<String, BTreeMap<String, Base64>>>,
	) -> Result<Option<Vec<u8>>> {
		// Skip the PDU if we already have it as a timeline event
		if let Ok(pduid) = self
			.services
			.timeline
			.get_pdu_id(&incoming_pdu.event_id)
			.await
		{
			return Ok(Some(pduid.to_vec()));
		}

		if self
			.services
			.pdu_metadata
			.is_event_soft_failed(&incoming_pdu.event_id)
			.await
		{
			return Err!(Request(InvalidParam("Event has been soft failed")));
		}

		debug!("Upgrading to timeline pdu");
		let timer = tokio::time::Instant::now();
		let room_version_id = Self::get_room_version_id(create_event)?;

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
		let room_version = Self::to_room_version(&room_version_id);

		debug!("Performing auth check");
		// 11. Check the auth of the event passes based on the state of the event
		let state_fetch_state = &state_at_incoming_event;
		let state_fetch = |k: &'static StateEventType, s: String| async move {
			let shortstatekey = self.services.short.get_shortstatekey(k, &s).await.ok()?;

			let event_id = state_fetch_state.get(&shortstatekey)?;
			self.services.timeline.get_pdu(event_id).await.ok()
		};

		let auth_check = state_res::event_auth::auth_check(
			&room_version,
			&incoming_pdu,
			None, // TODO: third party invite
			|k, s| state_fetch(k, s.to_owned()),
		)
		.await
		.map_err(|e| err!(Request(Forbidden("Auth check failed: {e:?}"))))?;

		if !auth_check {
			return Err!(Request(Forbidden("Event has failed auth check with state at the event.")));
		}

		debug!("Gathering auth events");
		let auth_events = self
			.services
			.state
			.get_auth_events(
				room_id,
				&incoming_pdu.kind,
				&incoming_pdu.sender,
				incoming_pdu.state_key.as_deref(),
				&incoming_pdu.content,
			)
			.await?;

		let state_fetch = |k: &'static StateEventType, s: &str| {
			let key = k.with_state_key(s);
			ready(auth_events.get(&key).cloned())
		};

		let auth_check = state_res::event_auth::auth_check(
			&room_version,
			&incoming_pdu,
			None, // third-party invite
			state_fetch,
		)
		.await
		.map_err(|e| err!(Request(Forbidden("Auth check failed: {e:?}"))))?;

		// Soft fail check before doing state res
		debug!("Performing soft-fail check");
		let soft_fail = {
			use RoomVersionId::*;

			!auth_check
				|| incoming_pdu.kind == TimelineEventType::RoomRedaction
					&& match room_version_id {
						V1 | V2 | V3 | V4 | V5 | V6 | V7 | V8 | V9 | V10 => {
							if let Some(redact_id) = &incoming_pdu.redacts {
								!self
									.services
									.state_accessor
									.user_can_redact(redact_id, &incoming_pdu.sender, &incoming_pdu.room_id, true)
									.await?
							} else {
								false
							}
						},
						_ => {
							let content = serde_json::from_str::<RoomRedactionEventContent>(incoming_pdu.content.get())
								.map_err(|_| Error::bad_database("Invalid content in redaction pdu."))?;

							if let Some(redact_id) = &content.redacts {
								!self
									.services
									.state_accessor
									.user_can_redact(redact_id, &incoming_pdu.sender, &incoming_pdu.room_id, true)
									.await?
							} else {
								false
							}
						},
					}
		};

		// 13. Use state resolution to find new room state

		// We start looking at current room state now, so lets lock the room
		trace!("Locking the room");
		let state_lock = self.services.state.mutex.lock(room_id).await;

		// Now we calculate the set of extremities this room has after the incoming
		// event has been applied. We start with the previous extremities (aka leaves)
		trace!("Calculating extremities");
		let mut extremities: HashSet<_> = self
			.services
			.state
			.get_forward_extremities(room_id)
			.map(ToOwned::to_owned)
			.collect()
			.await;

		// Remove any forward extremities that are referenced by this incoming event's
		// prev_events
		trace!(
			"Calculated {} extremities; checking against {} prev_events",
			extremities.len(),
			incoming_pdu.prev_events.len()
		);
		for prev_event in &incoming_pdu.prev_events {
			extremities.remove(&(**prev_event));
		}

		// Only keep those extremities were not referenced yet
		let mut retained = HashSet::new();
		for id in &extremities {
			if !self
				.services
				.pdu_metadata
				.is_event_referenced(room_id, id)
				.await
			{
				retained.insert(id.clone());
			}
		}

		extremities.retain(|id| retained.contains(id));
		debug!("Retained {} extremities. Compressing state", extremities.len());

		let mut state_ids_compressed = HashSet::new();
		for (shortstatekey, id) in &state_at_incoming_event {
			state_ids_compressed.insert(
				self.services
					.state_compressor
					.compress_state_event(*shortstatekey, id)
					.await,
			);
		}

		let state_ids_compressed = Arc::new(state_ids_compressed);

		if incoming_pdu.state_key.is_some() {
			debug!("Event is a state-event. Deriving new room state");

			// We also add state after incoming event to the fork states
			let mut state_after = state_at_incoming_event.clone();
			if let Some(state_key) = &incoming_pdu.state_key {
				let shortstatekey = self
					.services
					.short
					.get_or_create_shortstatekey(&incoming_pdu.kind.to_string().into(), state_key)
					.await;

				let event_id = &incoming_pdu.event_id;
				state_after.insert(shortstatekey, event_id.clone());
			}

			let new_room_state = self
				.resolve_state(room_id, &room_version_id, state_after)
				.await?;

			// Set the new room state to the resolved state
			debug!("Forcing new room state");
			let (sstatehash, new, removed) = self
				.services
				.state_compressor
				.save_state(room_id, new_room_state)
				.await?;

			self.services
				.state
				.force_state(room_id, sstatehash, new, removed, &state_lock)
				.await?;
		}

		// 14. Check if the event passes auth based on the "current state" of the room,
		//     if not soft fail it
		if soft_fail {
			debug!("Soft failing event");
			self.services
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
			warn!("Event was soft failed: {incoming_pdu:?}");
			self.services
				.pdu_metadata
				.mark_event_soft_failed(&incoming_pdu.event_id);

			return Err(Error::BadRequest(ErrorKind::InvalidParam, "Event has been soft failed"));
		}

		trace!("Appending pdu to timeline");
		extremities.insert(incoming_pdu.event_id.clone().into());

		// Now that the event has passed all auth it is added into the timeline.
		// We use the `state_at_event` instead of `state_after` so we accurately
		// represent the state for this event.
		let pdu_id = self
			.services
			.timeline
			.append_incoming_pdu(
				&incoming_pdu,
				val,
				extremities.into_iter().collect(),
				state_ids_compressed,
				soft_fail,
				&state_lock,
			)
			.await?;

		// Event has passed all auth/stateres checks
		drop(state_lock);
		debug_info!(
			elapsed = ?timer.elapsed(),
			"Accepted",
		);

		Ok(pdu_id)
	}

	#[tracing::instrument(skip_all, name = "resolve")]
	pub async fn resolve_state(
		&self, room_id: &RoomId, room_version_id: &RoomVersionId, incoming_state: HashMap<u64, Arc<EventId>>,
	) -> Result<Arc<HashSet<CompressedStateEvent>>> {
		debug!("Loading current room state ids");
		let current_sstatehash = self
			.services
			.state
			.get_room_shortstatehash(room_id)
			.await
			.map_err(|e| err!(Database(error!("No state for {room_id:?}: {e:?}"))))?;

		let current_state_ids = self
			.services
			.state_accessor
			.state_full_ids(current_sstatehash)
			.await?;

		let fork_states = [current_state_ids, incoming_state];
		let mut auth_chain_sets = Vec::with_capacity(fork_states.len());
		for state in &fork_states {
			let starting_events: Vec<&EventId> = state.values().map(Borrow::borrow).collect();

			let auth_chain = self
				.services
				.auth_chain
				.event_ids_iter(room_id, &starting_events)
				.await?
				.collect::<HashSet<Arc<EventId>>>()
				.await;

			auth_chain_sets.push(auth_chain);
		}

		debug!("Loading fork states");
		let fork_states: Vec<StateMap<Arc<EventId>>> = fork_states
			.into_iter()
			.stream()
			.then(|fork_state| {
				fork_state
					.into_iter()
					.stream()
					.filter_map(|(k, id)| {
						self.services
							.short
							.get_statekey_from_short(k)
							.map_ok_or_else(|_| None, move |(ty, st_key)| Some(((ty, st_key), id)))
					})
					.collect()
			})
			.collect()
			.boxed()
			.await;

		debug!("Resolving state");
		let lock = self.services.globals.stateres_mutex.lock();

		let event_fetch = |event_id| self.event_fetch(event_id);
		let event_exists = |event_id| self.event_exists(event_id);
		let state = state_res::resolve(room_version_id, &fork_states, &auth_chain_sets, &event_fetch, &event_exists)
			.await
			.map_err(|e| err!(Database(error!("State resolution failed: {e:?}"))))?;

		drop(lock);

		debug!("State resolution done. Compressing state");
		let mut new_room_state = HashSet::new();
		for ((event_type, state_key), event_id) in state {
			let shortstatekey = self
				.services
				.short
				.get_or_create_shortstatekey(&event_type.to_string().into(), &state_key)
				.await;

			let compressed = self
				.services
				.state_compressor
				.compress_state_event(shortstatekey, &event_id)
				.await;

			new_room_state.insert(compressed);
		}

		Ok(Arc::new(new_room_state))
	}

	// TODO: if we know the prev_events of the incoming event we can avoid the
	// request and build the state from a known point and resolve if > 1 prev_event
	#[tracing::instrument(skip_all, name = "state")]
	pub async fn state_at_incoming_degree_one(
		&self, incoming_pdu: &Arc<PduEvent>,
	) -> Result<Option<HashMap<u64, Arc<EventId>>>> {
		let prev_event = &*incoming_pdu.prev_events[0];
		let Ok(prev_event_sstatehash) = self
			.services
			.state_accessor
			.pdu_shortstatehash(prev_event)
			.await
		else {
			return Ok(None);
		};

		let Ok(mut state) = self
			.services
			.state_accessor
			.state_full_ids(prev_event_sstatehash)
			.await
			.log_err()
		else {
			return Ok(None);
		};

		debug!("Using cached state");
		let prev_pdu = self
			.services
			.timeline
			.get_pdu(prev_event)
			.await
			.map_err(|e| err!(Database("Could not find prev event, but we know the state: {e:?}")))?;

		if let Some(state_key) = &prev_pdu.state_key {
			let shortstatekey = self
				.services
				.short
				.get_or_create_shortstatekey(&prev_pdu.kind.to_string().into(), state_key)
				.await;

			state.insert(shortstatekey, Arc::from(prev_event));
			// Now it's the state after the pdu
		}

		debug_assert!(!state.is_empty(), "should be returning None for empty HashMap result");

		Ok(Some(state))
	}

	#[tracing::instrument(skip_all, name = "state")]
	pub async fn state_at_incoming_resolved(
		&self, incoming_pdu: &Arc<PduEvent>, room_id: &RoomId, room_version_id: &RoomVersionId,
	) -> Result<Option<HashMap<u64, Arc<EventId>>>> {
		debug!("Calculating state at event using state res");
		let mut extremity_sstatehashes = HashMap::with_capacity(incoming_pdu.prev_events.len());

		let mut okay = true;
		for prev_eventid in &incoming_pdu.prev_events {
			let Ok(prev_event) = self.services.timeline.get_pdu(prev_eventid).await else {
				okay = false;
				break;
			};

			let Ok(sstatehash) = self
				.services
				.state_accessor
				.pdu_shortstatehash(prev_eventid)
				.await
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
			let Ok(mut leaf_state) = self
				.services
				.state_accessor
				.state_full_ids(sstatehash)
				.await
			else {
				continue;
			};

			if let Some(state_key) = &prev_event.state_key {
				let shortstatekey = self
					.services
					.short
					.get_or_create_shortstatekey(&prev_event.kind.to_string().into(), state_key)
					.await;

				let event_id = &prev_event.event_id;
				leaf_state.insert(shortstatekey, event_id.clone());
				// Now it's the state after the pdu
			}

			let mut state = StateMap::with_capacity(leaf_state.len());
			let mut starting_events = Vec::with_capacity(leaf_state.len());
			for (k, id) in &leaf_state {
				if let Ok((ty, st_key)) = self
					.services
					.short
					.get_statekey_from_short(*k)
					.await
					.log_err()
				{
					// FIXME: Undo .to_string().into() when StateMap
					//        is updated to use StateEventType
					state.insert((ty.to_string().into(), st_key), id.clone());
				}

				starting_events.push(id.borrow());
			}

			let auth_chain = self
				.services
				.auth_chain
				.event_ids_iter(room_id, &starting_events)
				.await?
				.collect()
				.await;

			auth_chain_sets.push(auth_chain);
			fork_states.push(state);
		}

		let lock = self.services.globals.stateres_mutex.lock();

		let event_fetch = |event_id| self.event_fetch(event_id);
		let event_exists = |event_id| self.event_exists(event_id);
		let result = state_res::resolve(room_version_id, &fork_states, &auth_chain_sets, &event_fetch, &event_exists)
			.await
			.map_err(|e| err!(Database(warn!(?e, "State resolution on prev events failed."))));

		drop(lock);

		let Ok(new_state) = result else {
			return Ok(None);
		};

		new_state
			.iter()
			.stream()
			.then(|((event_type, state_key), event_id)| {
				self.services
					.short
					.get_or_create_shortstatekey(event_type, state_key)
					.map(move |shortstatekey| (shortstatekey, event_id.clone()))
			})
			.collect()
			.map(Some)
			.map(Ok)
			.await
	}

	/// Call /state_ids to find out what the state at this pdu is. We trust the
	/// server's response to some extend (sic), but we still do a lot of checks
	/// on the events
	#[tracing::instrument(skip(self, pub_key_map, create_event, room_version_id))]
	async fn fetch_state(
		&self, origin: &ServerName, create_event: &PduEvent, room_id: &RoomId, room_version_id: &RoomVersionId,
		pub_key_map: &RwLock<BTreeMap<String, BTreeMap<String, Base64>>>, event_id: &EventId,
	) -> Result<Option<HashMap<u64, Arc<EventId>>>> {
		debug!("Fetching state ids");
		let res = self
			.services
			.sending
			.send_federation_request(
				origin,
				get_room_state_ids::v1::Request {
					room_id: room_id.to_owned(),
					event_id: (*event_id).to_owned(),
				},
			)
			.await
			.inspect_err(|e| warn!("Fetching state for event failed: {e}"))?;

		debug!("Fetching state events");
		let collect = res
			.pdu_ids
			.iter()
			.map(|x| Arc::from(&**x))
			.collect::<Vec<_>>();

		let state_vec = self
			.fetch_and_handle_outliers(origin, &collect, create_event, room_id, room_version_id, pub_key_map)
			.boxed()
			.await;

		let mut state: HashMap<_, Arc<EventId>> = HashMap::with_capacity(state_vec.len());
		for (pdu, _) in state_vec {
			let state_key = pdu
				.state_key
				.clone()
				.ok_or_else(|| Error::bad_database("Found non-state pdu in state events."))?;

			let shortstatekey = self
				.services
				.short
				.get_or_create_shortstatekey(&pdu.kind.to_string().into(), &state_key)
				.await;

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
		let create_shortstatekey = self
			.services
			.short
			.get_shortstatekey(&StateEventType::RoomCreate, "")
			.await?;

		if state.get(&create_shortstatekey).map(AsRef::as_ref) != Some(&create_event.event_id) {
			return Err!(Database("Incoming event refers to wrong create event."));
		}

		Ok(Some(state))
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
	pub async fn fetch_and_handle_outliers<'a>(
		&self, origin: &'a ServerName, events: &'a [Arc<EventId>], create_event: &'a PduEvent, room_id: &'a RoomId,
		room_version_id: &'a RoomVersionId, pub_key_map: &'a RwLock<BTreeMap<String, BTreeMap<String, Base64>>>,
	) -> Vec<(Arc<PduEvent>, Option<BTreeMap<String, CanonicalJsonValue>>)> {
		let back_off = |id| match self
			.services
			.globals
			.bad_event_ratelimiter
			.write()
			.expect("locked")
			.entry(id)
		{
			hash_map::Entry::Vacant(e) => {
				e.insert((Instant::now(), 1));
			},
			hash_map::Entry::Occupied(mut e) => *e.get_mut() = (Instant::now(), e.get().1.saturating_add(1)),
		};

		let mut events_with_auth_events = Vec::with_capacity(events.len());
		for id in events {
			// a. Look in the main timeline (pduid_pdu tree)
			// b. Look at outlier pdu tree
			// (get_pdu_json checks both)
			if let Ok(local_pdu) = self.services.timeline.get_pdu(id).await {
				trace!("Found {id} in db");
				events_with_auth_events.push((id, Some(local_pdu), vec![]));
				continue;
			}

			// c. Ask origin server over federation
			// We also handle its auth chain here so we don't get a stack overflow in
			// handle_outlier_pdu.
			let mut todo_auth_events = vec![Arc::clone(id)];
			let mut events_in_reverse_order = Vec::with_capacity(todo_auth_events.len());
			let mut events_all = HashSet::with_capacity(todo_auth_events.len());
			let mut i: u64 = 0;
			while let Some(next_id) = todo_auth_events.pop() {
				if let Some((time, tries)) = self
					.services
					.globals
					.bad_event_ratelimiter
					.read()
					.expect("locked")
					.get(&*next_id)
				{
					// Exponential backoff
					const MIN_DURATION: u64 = 5 * 60;
					const MAX_DURATION: u64 = 60 * 60 * 24;
					if continue_exponential_backoff_secs(MIN_DURATION, MAX_DURATION, time.elapsed(), *tries) {
						info!("Backing off from {next_id}");
						continue;
					}
				}

				if events_all.contains(&next_id) {
					continue;
				}

				i = i.saturating_add(1);
				if i % 100 == 0 {
					tokio::task::yield_now().await;
				}

				if self.services.timeline.get_pdu(&next_id).await.is_ok() {
					trace!("Found {next_id} in db");
					continue;
				}

				debug!("Fetching {next_id} over federation.");
				match self
					.services
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
						debug!("Got {next_id} over federation");
						let Ok((calculated_event_id, value)) =
							pdu::gen_event_id_canonical_json(&res.pdu, room_version_id)
						else {
							back_off((*next_id).to_owned());
							continue;
						};

						if calculated_event_id != *next_id {
							warn!(
								"Server didn't return event id we requested: requested: {next_id}, we got \
								 {calculated_event_id}. Event: {:?}",
								&res.pdu
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
						debug_error!("Failed to fetch event {next_id}: {e}");
						back_off((*next_id).to_owned());
					},
				}
			}
			events_with_auth_events.push((id, None, events_in_reverse_order));
		}

		// We go through all the signatures we see on the PDUs and their unresolved
		// dependencies and fetch the corresponding signing keys
		self.services
			.server_keys
			.fetch_required_signing_keys(
				events_with_auth_events
					.iter()
					.flat_map(|(_id, _local_pdu, events)| events)
					.map(|(_event_id, event)| event),
				pub_key_map,
			)
			.await
			.unwrap_or_else(|e| {
				warn!("Could not fetch all signatures for PDUs from {origin}: {e:?}");
			});

		let mut pdus = Vec::with_capacity(events_with_auth_events.len());
		for (id, local_pdu, events_in_reverse_order) in events_with_auth_events {
			// a. Look in the main timeline (pduid_pdu tree)
			// b. Look at outlier pdu tree
			// (get_pdu_json checks both)
			if let Some(local_pdu) = local_pdu {
				trace!("Found {id} in db");
				pdus.push((local_pdu.clone(), None));
			}

			for (next_id, value) in events_in_reverse_order.into_iter().rev() {
				if let Some((time, tries)) = self
					.services
					.globals
					.bad_event_ratelimiter
					.read()
					.expect("locked")
					.get(&*next_id)
				{
					// Exponential backoff
					const MIN_DURATION: u64 = 5 * 60;
					const MAX_DURATION: u64 = 60 * 60 * 24;
					if continue_exponential_backoff_secs(MIN_DURATION, MAX_DURATION, time.elapsed(), *tries) {
						debug!("Backing off from {next_id}");
						continue;
					}
				}

				match Box::pin(self.handle_outlier_pdu(
					origin,
					create_event,
					&next_id,
					room_id,
					value.clone(),
					true,
					pub_key_map,
				))
				.await
				{
					Ok((pdu, json)) => {
						if next_id == *id {
							pdus.push((pdu, Some(json)));
						}
					},
					Err(e) => {
						warn!("Authentication of event {next_id} failed: {e:?}");
						back_off(next_id.into());
					},
				}
			}
		}
		pdus
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
		let mut graph: HashMap<Arc<EventId>, _> = HashMap::with_capacity(initial_set.len());
		let mut eventid_info = HashMap::new();
		let mut todo_outlier_stack: Vec<Arc<EventId>> = initial_set;

		let first_pdu_in_room = self.services.timeline.first_pdu_in_room(room_id).await?;

		let mut amount = 0;

		while let Some(prev_event_id) = todo_outlier_stack.pop() {
			if let Some((pdu, mut json_opt)) = self
				.fetch_and_handle_outliers(
					origin,
					&[prev_event_id.clone()],
					create_event,
					room_id,
					room_version_id,
					pub_key_map,
				)
				.boxed()
				.await
				.pop()
			{
				Self::check_room_id(room_id, &pdu)?;

				let limit = self.services.globals.max_fetch_prev_events();
				if amount > limit {
					debug_warn!("Max prev event limit reached! Limit: {limit}");
					graph.insert(prev_event_id.clone(), HashSet::new());
					continue;
				}

				if json_opt.is_none() {
					json_opt = self
						.services
						.outlier
						.get_outlier_pdu_json(&prev_event_id)
						.await
						.ok();
				}

				if let Some(json) = json_opt {
					if pdu.origin_server_ts > first_pdu_in_room.origin_server_ts {
						amount = amount.saturating_add(1);
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

		let event_fetch = |event_id| {
			let origin_server_ts = eventid_info
				.get(&event_id)
				.cloned()
				.map_or_else(|| uint!(0), |info| info.0.origin_server_ts);

			// This return value is the key used for sorting events,
			// events are then sorted by power level, time,
			// and lexically by event_id.
			future::ok((int!(0), MilliSecondsSinceUnixEpoch(origin_server_ts)))
		};

		let sorted = state_res::lexicographical_topological_sort(&graph, &event_fetch)
			.await
			.map_err(|e| err!(Database(error!("Error sorting prev events: {e}"))))?;

		Ok((sorted, eventid_info))
	}

	/// Returns Ok if the acl allows the server
	#[tracing::instrument(skip_all)]
	pub async fn acl_check(&self, server_name: &ServerName, room_id: &RoomId) -> Result<()> {
		let Ok(acl_event_content) = self
			.services
			.state_accessor
			.room_state_get_content(room_id, &StateEventType::RoomServerAcl, "")
			.await
			.map(|c: RoomServerAclEventContent| c)
			.inspect(|acl| trace!("ACL content found: {acl:?}"))
			.inspect_err(|e| trace!("No ACL content found: {e:?}"))
		else {
			return Ok(());
		};

		if acl_event_content.allow.is_empty() {
			warn!("Ignoring broken ACL event (allow key is empty)");
			return Ok(());
		}

		if acl_event_content.is_allowed(server_name) {
			trace!("server {server_name} is allowed by ACL");
			Ok(())
		} else {
			debug!("Server {server_name} was denied by room ACL in {room_id}");
			Err!(Request(Forbidden("Server was denied by room ACL")))
		}
	}

	fn check_room_id(room_id: &RoomId, pdu: &PduEvent) -> Result<()> {
		if pdu.room_id != room_id {
			return Err!(Request(InvalidParam(
				warn!(pdu_event_id = ?pdu.event_id, pdu_room_id = ?pdu.room_id, ?room_id, "Found event from room in room")
			)));
		}

		Ok(())
	}

	fn get_room_version_id(create_event: &PduEvent) -> Result<RoomVersionId> {
		let create_event_content: RoomCreateEventContent = serde_json::from_str(create_event.content.get())
			.map_err(|e| err!(Database("Invalid create event: {e}")))?;

		Ok(create_event_content.room_version)
	}

	#[inline]
	fn to_room_version(room_version_id: &RoomVersionId) -> RoomVersion {
		RoomVersion::new(room_version_id).expect("room version is supported")
	}

	async fn event_exists(&self, event_id: Arc<EventId>) -> bool { self.services.timeline.pdu_exists(&event_id).await }

	async fn event_fetch(&self, event_id: Arc<EventId>) -> Option<Arc<PduEvent>> {
		self.services.timeline.get_pdu(&event_id).await.ok()
	}
}
