use std::{
	cmp::{self},
	collections::{hash_map::Entry, BTreeMap, HashMap, HashSet},
	time::Duration,
};

use axum::extract::State;
use conduit::{
	err, error, extract_variant, is_equal_to,
	result::FlatOk,
	utils::{math::ruma_from_u64, BoolExt, IterStream, ReadyExt, TryFutureExtExt},
	PduCount,
};
use futures::{future::OptionFuture, pin_mut, FutureExt, StreamExt};
use ruma::{
	api::client::{
		filter::{FilterDefinition, LazyLoadOptions},
		sync::sync_events::{
			self,
			v3::{
				Ephemeral, Filter, GlobalAccountData, InviteState, InvitedRoom, JoinedRoom, LeftRoom, Presence,
				RoomAccountData, RoomSummary, Rooms, State as RoomState, Timeline, ToDevice,
			},
			DeviceLists, UnreadNotificationsCount,
		},
		uiaa::UiaaResponse,
	},
	events::{
		presence::PresenceEvent,
		room::member::{MembershipState, RoomMemberEventContent},
		AnyRawAccountDataEvent, StateEventType,
		TimelineEventType::*,
	},
	serde::Raw,
	uint, DeviceId, EventId, OwnedRoomId, OwnedUserId, RoomId, UserId,
};
use tracing::{Instrument as _, Span};

use super::{load_timeline, share_encrypted_room};
use crate::{
	service::{pdu::EventHash, Services},
	utils, Error, PduEvent, Result, Ruma, RumaResponse,
};

/// # `GET /_matrix/client/r0/sync`
///
/// Synchronize the client's state with the latest state on the server.
///
/// - This endpoint takes a `since` parameter which should be the `next_batch`
///   value from a previous request for incremental syncs.
///
/// Calling this endpoint without a `since` parameter returns:
/// - Some of the most recent events of each timeline
/// - Notification counts for each room
/// - Joined and invited member counts, heroes
/// - All state events
///
/// Calling this endpoint with a `since` parameter from a previous `next_batch`
/// returns: For joined rooms:
/// - Some of the most recent events of each timeline that happened after since
/// - If user joined the room after since: All state events (unless lazy loading
///   is activated) and all device list updates in that room
/// - If the user was already in the room: A list of all events that are in the
///   state now, but were not in the state at `since`
/// - If the state we send contains a member event: Joined and invited member
///   counts, heroes
/// - Device list updates that happened after `since`
/// - If there are events in the timeline we send or the user send updated his
///   read mark: Notification counts
/// - EDUs that are active now (read receipts, typing updates, presence)
/// - TODO: Allow multiple sync streams to support Pantalaimon
///
/// For invited rooms:
/// - If the user was invited after `since`: A subset of the state of the room
///   at the point of the invite
///
/// For left rooms:
/// - If the user left after `since`: `prev_batch` token, empty state (TODO:
///   subset of the state at the point of the leave)
pub(crate) async fn sync_events_route(
	State(services): State<crate::State>, body: Ruma<sync_events::v3::Request>,
) -> Result<sync_events::v3::Response, RumaResponse<UiaaResponse>> {
	let sender_user = body.sender_user.expect("user is authenticated");
	let sender_device = body.sender_device.expect("user is authenticated");
	let body = body.body;

	// Presence update
	if services.globals.allow_local_presence() {
		services
			.presence
			.ping_presence(&sender_user, &body.set_presence)
			.await?;
	}

	// Setup watchers, so if there's no response, we can wait for them
	let watcher = services.globals.watch(&sender_user, &sender_device);

	let next_batch = services.globals.current_count()?;
	let next_batchcount = PduCount::Normal(next_batch);
	let next_batch_string = next_batch.to_string();

	// Load filter
	let filter = match body.filter {
		None => FilterDefinition::default(),
		Some(Filter::FilterDefinition(filter)) => filter,
		Some(Filter::FilterId(filter_id)) => services
			.users
			.get_filter(&sender_user, &filter_id)
			.await
			.unwrap_or_default(),
	};

	// some clients, at least element, seem to require knowledge of redundant
	// members for "inline" profiles on the timeline to work properly
	let (lazy_load_enabled, lazy_load_send_redundant) = match filter.room.state.lazy_load_options {
		LazyLoadOptions::Enabled {
			include_redundant_members,
		} => (true, include_redundant_members),
		LazyLoadOptions::Disabled => (false, cfg!(feature = "element_hacks")),
	};

	let full_state = body.full_state;

	let mut joined_rooms = BTreeMap::new();
	let since = body
		.since
		.as_ref()
		.and_then(|string| string.parse().ok())
		.unwrap_or(0);
	let sincecount = PduCount::Normal(since);

	let mut presence_updates = HashMap::new();
	let mut left_encrypted_users = HashSet::new(); // Users that have left any encrypted rooms the sender was in
	let mut device_list_updates = HashSet::new();
	let mut device_list_left = HashSet::new();

	// Look for device list updates of this account
	device_list_updates.extend(
		services
			.users
			.keys_changed(sender_user.as_ref(), since, None)
			.map(ToOwned::to_owned)
			.collect::<Vec<_>>()
			.await,
	);

	if services.globals.allow_local_presence() {
		process_presence_updates(&services, &mut presence_updates, since, &sender_user).await?;
	}

	let all_joined_rooms: Vec<_> = services
		.rooms
		.state_cache
		.rooms_joined(&sender_user)
		.map(ToOwned::to_owned)
		.collect()
		.await;

	// Coalesce database writes for the remainder of this scope.
	let _cork = services.db.cork_and_flush();

	for room_id in all_joined_rooms {
		if let Ok(joined_room) = load_joined_room(
			&services,
			&sender_user,
			&sender_device,
			&room_id,
			since,
			sincecount,
			next_batch,
			next_batchcount,
			lazy_load_enabled,
			lazy_load_send_redundant,
			full_state,
			&mut device_list_updates,
			&mut left_encrypted_users,
		)
		.await
		{
			if !joined_room.is_empty() {
				joined_rooms.insert(room_id.clone(), joined_room);
			}
		}
	}

	let mut left_rooms = BTreeMap::new();
	let all_left_rooms: Vec<_> = services
		.rooms
		.state_cache
		.rooms_left(&sender_user)
		.collect()
		.await;

	for result in all_left_rooms {
		handle_left_room(
			&services,
			since,
			&result.0,
			&sender_user,
			&mut left_rooms,
			&next_batch_string,
			full_state,
			lazy_load_enabled,
		)
		.instrument(Span::current())
		.await?;
	}

	let mut invited_rooms = BTreeMap::new();
	let all_invited_rooms: Vec<_> = services
		.rooms
		.state_cache
		.rooms_invited(&sender_user)
		.collect()
		.await;

	for (room_id, invite_state_events) in all_invited_rooms {
		// Get and drop the lock to wait for remaining operations to finish
		let insert_lock = services.rooms.timeline.mutex_insert.lock(&room_id).await;
		drop(insert_lock);

		let invite_count = services
			.rooms
			.state_cache
			.get_invite_count(&room_id, &sender_user)
			.await
			.ok();

		// Invited before last sync
		if Some(since) >= invite_count {
			continue;
		}

		invited_rooms.insert(
			room_id.clone(),
			InvitedRoom {
				invite_state: InviteState {
					events: invite_state_events,
				},
			},
		);
	}

	for user_id in left_encrypted_users {
		let dont_share_encrypted_room = !share_encrypted_room(&services, &sender_user, &user_id, None).await;

		// If the user doesn't share an encrypted room with the target anymore, we need
		// to tell them
		if dont_share_encrypted_room {
			device_list_left.insert(user_id);
		}
	}

	// Remove all to-device events the device received *last time*
	services
		.users
		.remove_to_device_events(&sender_user, &sender_device, since)
		.await;

	let response = sync_events::v3::Response {
		next_batch: next_batch_string,
		rooms: Rooms {
			leave: left_rooms,
			join: joined_rooms,
			invite: invited_rooms,
			knock: BTreeMap::new(), // TODO
		},
		presence: Presence {
			events: presence_updates
				.into_values()
				.map(|v| Raw::new(&v).expect("PresenceEvent always serializes successfully"))
				.collect(),
		},
		account_data: GlobalAccountData {
			events: services
				.account_data
				.changes_since(None, &sender_user, since)
				.await?
				.into_iter()
				.filter_map(|e| extract_variant!(e, AnyRawAccountDataEvent::Global))
				.collect(),
		},
		device_lists: DeviceLists {
			changed: device_list_updates.into_iter().collect(),
			left: device_list_left.into_iter().collect(),
		},
		device_one_time_keys_count: services
			.users
			.count_one_time_keys(&sender_user, &sender_device)
			.await,
		to_device: ToDevice {
			events: services
				.users
				.get_to_device_events(&sender_user, &sender_device)
				.collect()
				.await,
		},
		// Fallback keys are not yet supported
		device_unused_fallback_key_types: None,
	};

	// TODO: Retry the endpoint instead of returning
	if !full_state
		&& response.rooms.is_empty()
		&& response.presence.is_empty()
		&& response.account_data.is_empty()
		&& response.device_lists.is_empty()
		&& response.to_device.is_empty()
	{
		// Hang a few seconds so requests are not spammed
		// Stop hanging if new info arrives
		let default = Duration::from_secs(30);
		let duration = cmp::min(body.timeout.unwrap_or(default), default);
		_ = tokio::time::timeout(duration, watcher).await;
	}

	Ok(response)
}

#[allow(clippy::too_many_arguments)]
#[tracing::instrument(skip_all, fields(user_id = %sender_user, room_id = %room_id), name = "left_room")]
async fn handle_left_room(
	services: &Services, since: u64, room_id: &RoomId, sender_user: &UserId,
	left_rooms: &mut BTreeMap<OwnedRoomId, LeftRoom>, next_batch_string: &str, full_state: bool,
	lazy_load_enabled: bool,
) -> Result<()> {
	// Get and drop the lock to wait for remaining operations to finish
	let insert_lock = services.rooms.timeline.mutex_insert.lock(room_id).await;
	drop(insert_lock);

	let left_count = services
		.rooms
		.state_cache
		.get_left_count(room_id, sender_user)
		.await
		.ok();

	// Left before last sync
	if Some(since) >= left_count {
		return Ok(());
	}

	if !services.rooms.metadata.exists(room_id).await {
		// This is just a rejected invite, not a room we know
		// Insert a leave event anyways
		let event = PduEvent {
			event_id: EventId::new(services.globals.server_name()).into(),
			sender: sender_user.to_owned(),
			origin: None,
			origin_server_ts: utils::millis_since_unix_epoch()
				.try_into()
				.expect("Timestamp is valid js_int value"),
			kind: RoomMember,
			content: serde_json::from_str(r#"{"membership":"leave"}"#).expect("this is valid JSON"),
			state_key: Some(sender_user.to_string()),
			unsigned: None,
			// The following keys are dropped on conversion
			room_id: room_id.to_owned(),
			prev_events: vec![],
			depth: uint!(1),
			auth_events: vec![],
			redacts: None,
			hashes: EventHash {
				sha256: String::new(),
			},
			signatures: None,
		};

		left_rooms.insert(
			room_id.to_owned(),
			LeftRoom {
				account_data: RoomAccountData {
					events: Vec::new(),
				},
				timeline: Timeline {
					limited: false,
					prev_batch: Some(next_batch_string.to_owned()),
					events: Vec::new(),
				},
				state: RoomState {
					events: vec![event.to_sync_state_event()],
				},
			},
		);
		return Ok(());
	}

	let mut left_state_events = Vec::new();

	let since_shortstatehash = services
		.rooms
		.user
		.get_token_shortstatehash(room_id, since)
		.await;

	let since_state_ids = match since_shortstatehash {
		Ok(s) => services.rooms.state_accessor.state_full_ids(s).await?,
		Err(_) => HashMap::new(),
	};

	let Ok(left_event_id) = services
		.rooms
		.state_accessor
		.room_state_get_id(room_id, &StateEventType::RoomMember, sender_user.as_str())
		.await
	else {
		error!("Left room but no left state event");
		return Ok(());
	};

	let Ok(left_shortstatehash) = services
		.rooms
		.state_accessor
		.pdu_shortstatehash(&left_event_id)
		.await
	else {
		error!(event_id = %left_event_id, "Leave event has no state");
		return Ok(());
	};

	let mut left_state_ids = services
		.rooms
		.state_accessor
		.state_full_ids(left_shortstatehash)
		.await?;

	let leave_shortstatekey = services
		.rooms
		.short
		.get_or_create_shortstatekey(&StateEventType::RoomMember, sender_user.as_str())
		.await;

	left_state_ids.insert(leave_shortstatekey, left_event_id);

	let mut i: u8 = 0;
	for (key, id) in left_state_ids {
		if full_state || since_state_ids.get(&key) != Some(&id) {
			let (event_type, state_key) = services.rooms.short.get_statekey_from_short(key).await?;

			if !lazy_load_enabled
                    || event_type != StateEventType::RoomMember
                    || full_state
                    // TODO: Delete the following line when this is resolved: https://github.com/vector-im/element-web/issues/22565
                    || (cfg!(feature = "element_hacks") && *sender_user == state_key)
			{
				let Ok(pdu) = services.rooms.timeline.get_pdu(&id).await else {
					error!("Pdu in state not found: {}", id);
					continue;
				};

				left_state_events.push(pdu.to_sync_state_event());

				i = i.wrapping_add(1);
				if i % 100 == 0 {
					tokio::task::yield_now().await;
				}
			}
		}
	}

	left_rooms.insert(
		room_id.to_owned(),
		LeftRoom {
			account_data: RoomAccountData {
				events: Vec::new(),
			},
			timeline: Timeline {
				limited: false,
				prev_batch: Some(next_batch_string.to_owned()),
				events: Vec::new(),
			},
			state: RoomState {
				events: left_state_events,
			},
		},
	);
	Ok(())
}

async fn process_presence_updates(
	services: &Services, presence_updates: &mut HashMap<OwnedUserId, PresenceEvent>, since: u64, syncing_user: &UserId,
) -> Result<()> {
	let presence_since = services.presence.presence_since(since);

	// Take presence updates
	pin_mut!(presence_since);
	while let Some((user_id, _, presence_bytes)) = presence_since.next().await {
		if !services
			.rooms
			.state_cache
			.user_sees_user(syncing_user, user_id)
			.await
		{
			continue;
		}

		let presence_event = services
			.presence
			.from_json_bytes_to_event(presence_bytes, user_id)
			.await?;

		match presence_updates.entry(user_id.into()) {
			Entry::Vacant(slot) => {
				slot.insert(presence_event);
			},
			Entry::Occupied(mut slot) => {
				let curr_event = slot.get_mut();
				let curr_content = &mut curr_event.content;
				let new_content = presence_event.content;

				// Update existing presence event with more info
				curr_content.presence = new_content.presence;
				curr_content.status_msg = new_content
					.status_msg
					.or_else(|| curr_content.status_msg.take());
				curr_content.last_active_ago = new_content.last_active_ago.or(curr_content.last_active_ago);
				curr_content.displayname = new_content
					.displayname
					.or_else(|| curr_content.displayname.take());
				curr_content.avatar_url = new_content
					.avatar_url
					.or_else(|| curr_content.avatar_url.take());
				curr_content.currently_active = new_content
					.currently_active
					.or(curr_content.currently_active);
			},
		};
	}

	Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn load_joined_room(
	services: &Services, sender_user: &UserId, sender_device: &DeviceId, room_id: &RoomId, since: u64,
	sincecount: PduCount, next_batch: u64, next_batchcount: PduCount, lazy_load_enabled: bool,
	lazy_load_send_redundant: bool, full_state: bool, device_list_updates: &mut HashSet<OwnedUserId>,
	left_encrypted_users: &mut HashSet<OwnedUserId>,
) -> Result<JoinedRoom> {
	// Get and drop the lock to wait for remaining operations to finish
	// This will make sure the we have all events until next_batch
	let insert_lock = services.rooms.timeline.mutex_insert.lock(room_id).await;
	drop(insert_lock);

	let (timeline_pdus, limited) = load_timeline(services, sender_user, room_id, sincecount, 10).await?;

	let send_notification_counts = !timeline_pdus.is_empty()
		|| services
			.rooms
			.user
			.last_notification_read(sender_user, room_id)
			.await > since;

	let mut timeline_users = HashSet::new();
	for (_, event) in &timeline_pdus {
		timeline_users.insert(event.sender.as_str().to_owned());
	}

	services
		.rooms
		.lazy_loading
		.lazy_load_confirm_delivery(sender_user, sender_device, room_id, sincecount);

	// Database queries:

	let current_shortstatehash = services
		.rooms
		.state
		.get_room_shortstatehash(room_id)
		.await
		.map_err(|_| err!(Database(error!("Room {room_id} has no state"))))?;

	let since_shortstatehash = services
		.rooms
		.user
		.get_token_shortstatehash(room_id, since)
		.await
		.ok();

	let (heroes, joined_member_count, invited_member_count, joined_since_last_sync, state_events) = if timeline_pdus
		.is_empty()
		&& (since_shortstatehash.is_none() || since_shortstatehash.is_some_and(is_equal_to!(current_shortstatehash)))
	{
		// No state changes
		(Vec::new(), None, None, false, Vec::new())
	} else {
		// Calculates joined_member_count, invited_member_count and heroes
		let calculate_counts = || async {
			let joined_member_count = services
				.rooms
				.state_cache
				.room_joined_count(room_id)
				.await
				.unwrap_or(0);

			let invited_member_count = services
				.rooms
				.state_cache
				.room_invited_count(room_id)
				.await
				.unwrap_or(0);

			if joined_member_count.saturating_add(invited_member_count) > 5 {
				return Ok::<_, Error>((Some(joined_member_count), Some(invited_member_count), Vec::new()));
			}

			// Go through all PDUs and for each member event, check if the user is still
			// joined or invited until we have 5 or we reach the end

			// Recalculate heroes (first 5 members)
			let heroes = services
				.rooms
				.timeline
				.all_pdus(sender_user, room_id)
				.await?
				.ready_filter(|(_, pdu)| pdu.kind == RoomMember)
				.filter_map(|(_, pdu)| async move {
					let content: RoomMemberEventContent = pdu.get_content().ok()?;
					let user_id: &UserId = pdu.state_key.as_deref().map(TryInto::try_into).flat_ok()?;

					if user_id == sender_user {
						return None;
					}

					// The membership was and still is invite or join
					if !matches!(content.membership, MembershipState::Join | MembershipState::Invite) {
						return None;
					}

					let is_invited = services.rooms.state_cache.is_invited(user_id, room_id);

					let is_joined = services.rooms.state_cache.is_joined(user_id, room_id);

					if !is_joined.await && is_invited.await {
						return None;
					}

					Some(user_id.to_owned())
				})
				.collect::<HashSet<OwnedUserId>>()
				.await;

			Ok::<_, Error>((
				Some(joined_member_count),
				Some(invited_member_count),
				heroes.into_iter().collect::<Vec<_>>(),
			))
		};

		let get_sender_member_content = |short| {
			services
				.rooms
				.state_accessor
				.state_get_content(short, &StateEventType::RoomMember, sender_user.as_str())
				.ok()
		};

		let since_sender_member: OptionFuture<_> = since_shortstatehash.map(get_sender_member_content).into();

		let joined_since_last_sync = since_sender_member
			.await
			.flatten()
			.map_or(true, |content: RoomMemberEventContent| {
				content.membership != MembershipState::Join
			});

		if since_shortstatehash.is_none() || joined_since_last_sync {
			// Probably since = 0, we will do an initial sync

			let (joined_member_count, invited_member_count, heroes) = calculate_counts().await?;

			let current_state_ids = services
				.rooms
				.state_accessor
				.state_full_ids(current_shortstatehash)
				.await?;

			let mut state_events = Vec::new();
			let mut lazy_loaded = HashSet::new();

			let mut i: u8 = 0;
			for (shortstatekey, id) in current_state_ids {
				let (event_type, state_key) = services
					.rooms
					.short
					.get_statekey_from_short(shortstatekey)
					.await?;

				if event_type != StateEventType::RoomMember {
					let Ok(pdu) = services.rooms.timeline.get_pdu(&id).await else {
						error!("Pdu in state not found: {id}");
						continue;
					};
					state_events.push(pdu);

					i = i.wrapping_add(1);
					if i % 100 == 0 {
						tokio::task::yield_now().await;
					}
				} else if !lazy_load_enabled
                || full_state
                || timeline_users.contains(&state_key)
                // TODO: Delete the following line when this is resolved: https://github.com/vector-im/element-web/issues/22565
                || (cfg!(feature = "element_hacks") && *sender_user == state_key)
				{
					let Ok(pdu) = services.rooms.timeline.get_pdu(&id).await else {
						error!("Pdu in state not found: {id}");
						continue;
					};

					// This check is in case a bad user ID made it into the database
					if let Ok(uid) = UserId::parse(&state_key) {
						lazy_loaded.insert(uid);
					}
					state_events.push(pdu);

					i = i.wrapping_add(1);
					if i % 100 == 0 {
						tokio::task::yield_now().await;
					}
				}
			}

			// Reset lazy loading because this is an initial sync
			services
				.rooms
				.lazy_loading
				.lazy_load_reset(sender_user, sender_device, room_id)
				.await;

			// The state_events above should contain all timeline_users, let's mark them as
			// lazy loaded.
			services.rooms.lazy_loading.lazy_load_mark_sent(
				sender_user,
				sender_device,
				room_id,
				lazy_loaded,
				next_batchcount,
			);

			(heroes, joined_member_count, invited_member_count, true, state_events)
		} else {
			// Incremental /sync
			let since_shortstatehash = since_shortstatehash.expect("missing since_shortstatehash on incremental sync");

			let mut delta_state_events = Vec::new();

			if since_shortstatehash != current_shortstatehash {
				let current_state_ids = services
					.rooms
					.state_accessor
					.state_full_ids(current_shortstatehash)
					.await?;

				let since_state_ids = services
					.rooms
					.state_accessor
					.state_full_ids(since_shortstatehash)
					.await?;

				for (key, id) in current_state_ids {
					if full_state || since_state_ids.get(&key) != Some(&id) {
						let Ok(pdu) = services.rooms.timeline.get_pdu(&id).await else {
							error!("Pdu in state not found: {id}");
							continue;
						};

						delta_state_events.push(pdu);
						tokio::task::yield_now().await;
					}
				}
			}

			let encrypted_room = services
				.rooms
				.state_accessor
				.state_get(current_shortstatehash, &StateEventType::RoomEncryption, "")
				.await
				.is_ok();

			let since_encryption = services
				.rooms
				.state_accessor
				.state_get(since_shortstatehash, &StateEventType::RoomEncryption, "")
				.await;

			// Calculations:
			let new_encrypted_room = encrypted_room && since_encryption.is_err();

			let send_member_count = delta_state_events
				.iter()
				.any(|event| event.kind == RoomMember);

			if encrypted_room {
				for state_event in &delta_state_events {
					if state_event.kind != RoomMember {
						continue;
					}

					if let Some(state_key) = &state_event.state_key {
						let user_id = UserId::parse(state_key.clone())
							.map_err(|_| Error::bad_database("Invalid UserId in member PDU."))?;

						if user_id == sender_user {
							continue;
						}

						let content: RoomMemberEventContent = state_event.get_content()?;

						match content.membership {
							MembershipState::Join => {
								// A new user joined an encrypted room
								if !share_encrypted_room(services, sender_user, &user_id, Some(room_id)).await {
									device_list_updates.insert(user_id);
								}
							},
							MembershipState::Leave => {
								// Write down users that have left encrypted rooms we are in
								left_encrypted_users.insert(user_id);
							},
							_ => {},
						}
					}
				}
			}

			if joined_since_last_sync && encrypted_room || new_encrypted_room {
				// If the user is in a new encrypted room, give them all joined users
				device_list_updates.extend(
					services
						.rooms
						.state_cache
						.room_members(room_id)
						// Don't send key updates from the sender to the sender
						.ready_filter(|user_id| sender_user != *user_id)
						// Only send keys if the sender doesn't share an encrypted room with the target
						// already
						.filter_map(|user_id| {
							share_encrypted_room(services, sender_user, user_id, Some(room_id))
								.map(|res| res.or_some(user_id.to_owned()))
						})
						.collect::<Vec<_>>()
						.await,
				);
			}

			let (joined_member_count, invited_member_count, heroes) = if send_member_count {
				calculate_counts().await?
			} else {
				(None, None, Vec::new())
			};

			let mut state_events = delta_state_events;
			let mut lazy_loaded = HashSet::new();

			// Mark all member events we're returning as lazy-loaded
			for pdu in &state_events {
				if pdu.kind == RoomMember {
					match UserId::parse(
						pdu.state_key
							.as_ref()
							.expect("State event has state key")
							.clone(),
					) {
						Ok(state_key_userid) => {
							lazy_loaded.insert(state_key_userid);
						},
						Err(e) => error!("Invalid state key for member event: {}", e),
					}
				}
			}

			// Fetch contextual member state events for events from the timeline, and
			// mark them as lazy-loaded as well.
			for (_, event) in &timeline_pdus {
				if lazy_loaded.contains(&event.sender) {
					continue;
				}

				if !services
					.rooms
					.lazy_loading
					.lazy_load_was_sent_before(sender_user, sender_device, room_id, &event.sender)
					.await || lazy_load_send_redundant
				{
					if let Ok(member_event) = services
						.rooms
						.state_accessor
						.room_state_get(room_id, &StateEventType::RoomMember, event.sender.as_str())
						.await
					{
						lazy_loaded.insert(event.sender.clone());
						state_events.push(member_event);
					}
				}
			}

			services.rooms.lazy_loading.lazy_load_mark_sent(
				sender_user,
				sender_device,
				room_id,
				lazy_loaded,
				next_batchcount,
			);

			(
				heroes,
				joined_member_count,
				invited_member_count,
				joined_since_last_sync,
				state_events,
			)
		}
	};

	// Look for device list updates in this room
	device_list_updates.extend(
		services
			.users
			.keys_changed(room_id.as_ref(), since, None)
			.map(ToOwned::to_owned)
			.collect::<Vec<_>>()
			.await,
	);

	let notification_count = if send_notification_counts {
		Some(
			services
				.rooms
				.user
				.notification_count(sender_user, room_id)
				.await
				.try_into()
				.expect("notification count can't go that high"),
		)
	} else {
		None
	};

	let highlight_count = if send_notification_counts {
		Some(
			services
				.rooms
				.user
				.highlight_count(sender_user, room_id)
				.await
				.try_into()
				.expect("highlight count can't go that high"),
		)
	} else {
		None
	};

	let prev_batch = timeline_pdus
		.first()
		.map_or(Ok::<_, Error>(None), |(pdu_count, _)| {
			Ok(Some(match pdu_count {
				PduCount::Backfilled(_) => {
					error!("timeline in backfill state?!");
					"0".to_owned()
				},
				PduCount::Normal(c) => c.to_string(),
			}))
		})?;

	let room_events: Vec<_> = timeline_pdus
		.iter()
		.stream()
		.filter_map(|(_, pdu)| async move {
			// list of safe and common non-state events to ignore
			if matches!(
				&pdu.kind,
				RoomMessage
					| Sticker | CallInvite
					| CallNotify | RoomEncrypted
					| Image | File | Audio
					| Voice | Video | UnstablePollStart
					| PollStart | KeyVerificationStart
					| Reaction | Emote
					| Location
			) && services
				.users
				.user_is_ignored(&pdu.sender, sender_user)
				.await
			{
				return None;
			}

			Some(pdu.to_sync_room_event())
		})
		.collect()
		.await;

	let mut edus: Vec<_> = services
		.rooms
		.read_receipt
		.readreceipts_since(room_id, since)
		.filter_map(|(read_user, _, v)| async move {
			(!services
				.users
				.user_is_ignored(&read_user, sender_user)
				.await)
				.then_some(v)
		})
		.collect()
		.await;

	if services.rooms.typing.last_typing_update(room_id).await? > since {
		edus.push(
			serde_json::from_str(
				&serde_json::to_string(
					&services
						.rooms
						.typing
						.typings_all(room_id, sender_user)
						.await?,
				)
				.expect("event is valid, we just created it"),
			)
			.expect("event is valid, we just created it"),
		);
	}

	// Save the state after this sync so we can send the correct state diff next
	// sync
	services
		.rooms
		.user
		.associate_token_shortstatehash(room_id, next_batch, current_shortstatehash)
		.await;

	Ok(JoinedRoom {
		account_data: RoomAccountData {
			events: services
				.account_data
				.changes_since(Some(room_id), sender_user, since)
				.await?
				.into_iter()
				.filter_map(|e| extract_variant!(e, AnyRawAccountDataEvent::Room))
				.collect(),
		},
		summary: RoomSummary {
			heroes,
			joined_member_count: joined_member_count.map(ruma_from_u64),
			invited_member_count: invited_member_count.map(ruma_from_u64),
		},
		unread_notifications: UnreadNotificationsCount {
			highlight_count,
			notification_count,
		},
		timeline: Timeline {
			limited: limited || joined_since_last_sync,
			prev_batch,
			events: room_events,
		},
		state: RoomState {
			events: state_events
				.iter()
				.map(|pdu| pdu.to_sync_state_event())
				.collect(),
		},
		ephemeral: Ephemeral {
			events: edus,
		},
		unread_thread_notifications: BTreeMap::new(),
	})
}
