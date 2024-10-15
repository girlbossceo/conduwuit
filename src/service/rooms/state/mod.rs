use std::{
	collections::{HashMap, HashSet},
	fmt::Write,
	iter::once,
	sync::Arc,
};

use conduit::{
	err,
	result::FlatOk,
	utils::{calculate_hash, stream::TryIgnore, IterStream, MutexMap, MutexMapGuard, ReadyExt},
	warn, PduEvent, Result,
};
use database::{Deserialized, Ignore, Interfix, Map};
use futures::{future::join_all, pin_mut, FutureExt, Stream, StreamExt, TryFutureExt, TryStreamExt};
use ruma::{
	events::{
		room::{create::RoomCreateEventContent, member::RoomMemberEventContent},
		AnyStrippedStateEvent, StateEventType, TimelineEventType,
	},
	serde::Raw,
	state_res::{self, StateMap},
	EventId, OwnedEventId, OwnedRoomId, RoomId, RoomVersionId, UserId,
};

use super::state_compressor::CompressedStateEvent;
use crate::{globals, rooms, Dep};

pub struct Service {
	pub mutex: RoomMutexMap,
	services: Services,
	db: Data,
}

struct Services {
	globals: Dep<globals::Service>,
	short: Dep<rooms::short::Service>,
	spaces: Dep<rooms::spaces::Service>,
	state_cache: Dep<rooms::state_cache::Service>,
	state_accessor: Dep<rooms::state_accessor::Service>,
	state_compressor: Dep<rooms::state_compressor::Service>,
	timeline: Dep<rooms::timeline::Service>,
}

struct Data {
	shorteventid_shortstatehash: Arc<Map>,
	roomid_shortstatehash: Arc<Map>,
	roomid_pduleaves: Arc<Map>,
}

type RoomMutexMap = MutexMap<OwnedRoomId, ()>;
pub type RoomMutexGuard = MutexMapGuard<OwnedRoomId, ()>;

impl crate::Service for Service {
	fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
		Ok(Arc::new(Self {
			mutex: RoomMutexMap::new(),
			services: Services {
				globals: args.depend::<globals::Service>("globals"),
				short: args.depend::<rooms::short::Service>("rooms::short"),
				spaces: args.depend::<rooms::spaces::Service>("rooms::spaces"),
				state_cache: args.depend::<rooms::state_cache::Service>("rooms::state_cache"),
				state_accessor: args.depend::<rooms::state_accessor::Service>("rooms::state_accessor"),
				state_compressor: args.depend::<rooms::state_compressor::Service>("rooms::state_compressor"),
				timeline: args.depend::<rooms::timeline::Service>("rooms::timeline"),
			},
			db: Data {
				shorteventid_shortstatehash: args.db["shorteventid_shortstatehash"].clone(),
				roomid_shortstatehash: args.db["roomid_shortstatehash"].clone(),
				roomid_pduleaves: args.db["roomid_pduleaves"].clone(),
			},
		}))
	}

	fn memory_usage(&self, out: &mut dyn Write) -> Result {
		let mutex = self.mutex.len();
		writeln!(out, "state_mutex: {mutex}")?;

		Ok(())
	}

	fn name(&self) -> &str { crate::service::make_name(std::module_path!()) }
}

impl Service {
	/// Set the room to the given statehash and update caches.
	pub async fn force_state(
		&self,
		room_id: &RoomId,
		shortstatehash: u64,
		statediffnew: Arc<HashSet<CompressedStateEvent>>,
		_statediffremoved: Arc<HashSet<CompressedStateEvent>>,
		state_lock: &RoomMutexGuard, // Take mutex guard to make sure users get the room state mutex
	) -> Result {
		let event_ids = statediffnew.iter().stream().filter_map(|new| {
			self.services
				.state_compressor
				.parse_compressed_state_event(new)
				.map_ok_or_else(|_| None, |(_, event_id)| Some(event_id))
		});

		pin_mut!(event_ids);
		while let Some(event_id) = event_ids.next().await {
			let Ok(pdu) = self.services.timeline.get_pdu(&event_id).await else {
				continue;
			};

			match pdu.kind {
				TimelineEventType::RoomMember => {
					let Some(user_id) = pdu.state_key.as_ref().map(UserId::parse).flat_ok() else {
						continue;
					};

					let Ok(membership_event) = pdu.get_content::<RoomMemberEventContent>() else {
						continue;
					};

					self.services
						.state_cache
						.update_membership(room_id, &user_id, membership_event, &pdu.sender, None, None, false)
						.await?;
				},
				TimelineEventType::SpaceChild => {
					self.services
						.spaces
						.roomid_spacehierarchy_cache
						.lock()
						.await
						.remove(&pdu.room_id);
				},
				_ => continue,
			}
		}

		self.services.state_cache.update_joined_count(room_id).await;

		self.set_room_state(room_id, shortstatehash, state_lock);

		Ok(())
	}

	/// Generates a new StateHash and associates it with the incoming event.
	///
	/// This adds all current state events (not including the incoming event)
	/// to `stateid_pduid` and adds the incoming event to `eventid_statehash`.
	#[tracing::instrument(skip(self, state_ids_compressed), level = "debug")]
	pub async fn set_event_state(
		&self, event_id: &EventId, room_id: &RoomId, state_ids_compressed: Arc<HashSet<CompressedStateEvent>>,
	) -> Result<u64> {
		const BUFSIZE: usize = size_of::<u64>();

		let shorteventid = self
			.services
			.short
			.get_or_create_shorteventid(event_id)
			.await;

		let previous_shortstatehash = self.get_room_shortstatehash(room_id).await;

		let state_hash = calculate_hash(
			&state_ids_compressed
				.iter()
				.map(|s| &s[..])
				.collect::<Vec<_>>(),
		);

		let (shortstatehash, already_existed) = self
			.services
			.short
			.get_or_create_shortstatehash(&state_hash)
			.await;

		if !already_existed {
			let states_parents = if let Ok(p) = previous_shortstatehash {
				self.services
					.state_compressor
					.load_shortstatehash_info(p)
					.await?
			} else {
				Vec::new()
			};

			let (statediffnew, statediffremoved) = if let Some(parent_stateinfo) = states_parents.last() {
				let statediffnew: HashSet<_> = state_ids_compressed
					.difference(&parent_stateinfo.1)
					.copied()
					.collect();

				let statediffremoved: HashSet<_> = parent_stateinfo
					.1
					.difference(&state_ids_compressed)
					.copied()
					.collect();

				(Arc::new(statediffnew), Arc::new(statediffremoved))
			} else {
				(state_ids_compressed, Arc::new(HashSet::new()))
			};
			self.services.state_compressor.save_state_from_diff(
				shortstatehash,
				statediffnew,
				statediffremoved,
				1_000_000, // high number because no state will be based on this one
				states_parents,
			)?;
		}

		self.db
			.shorteventid_shortstatehash
			.aput::<BUFSIZE, BUFSIZE, _, _>(shorteventid, shortstatehash);

		Ok(shortstatehash)
	}

	/// Generates a new StateHash and associates it with the incoming event.
	///
	/// This adds all current state events (not including the incoming event)
	/// to `stateid_pduid` and adds the incoming event to `eventid_statehash`.
	#[tracing::instrument(skip(self, new_pdu), level = "debug")]
	pub async fn append_to_state(&self, new_pdu: &PduEvent) -> Result<u64> {
		const BUFSIZE: usize = size_of::<u64>();

		let shorteventid = self
			.services
			.short
			.get_or_create_shorteventid(&new_pdu.event_id)
			.await;

		let previous_shortstatehash = self.get_room_shortstatehash(&new_pdu.room_id).await;

		if let Ok(p) = previous_shortstatehash {
			self.db
				.shorteventid_shortstatehash
				.aput::<BUFSIZE, BUFSIZE, _, _>(shorteventid, p);
		}

		if let Some(state_key) = &new_pdu.state_key {
			let states_parents = if let Ok(p) = previous_shortstatehash {
				self.services
					.state_compressor
					.load_shortstatehash_info(p)
					.await?
			} else {
				Vec::new()
			};

			let shortstatekey = self
				.services
				.short
				.get_or_create_shortstatekey(&new_pdu.kind.to_string().into(), state_key)
				.await;

			let new = self
				.services
				.state_compressor
				.compress_state_event(shortstatekey, &new_pdu.event_id)
				.await;

			let replaces = states_parents
				.last()
				.map(|info| {
					info.1
						.iter()
						.find(|bytes| bytes.starts_with(&shortstatekey.to_be_bytes()))
				})
				.unwrap_or_default();

			if Some(&new) == replaces {
				return Ok(previous_shortstatehash.expect("must exist"));
			}

			// TODO: statehash with deterministic inputs
			let shortstatehash = self.services.globals.next_count()?;

			let mut statediffnew = HashSet::new();
			statediffnew.insert(new);

			let mut statediffremoved = HashSet::new();
			if let Some(replaces) = replaces {
				statediffremoved.insert(*replaces);
			}

			self.services.state_compressor.save_state_from_diff(
				shortstatehash,
				Arc::new(statediffnew),
				Arc::new(statediffremoved),
				2,
				states_parents,
			)?;

			Ok(shortstatehash)
		} else {
			Ok(previous_shortstatehash.expect("first event in room must be a state event"))
		}
	}

	#[tracing::instrument(skip_all, level = "debug")]
	pub async fn summary_stripped(&self, invite: &PduEvent) -> Vec<Raw<AnyStrippedStateEvent>> {
		let cells = [
			(&StateEventType::RoomCreate, ""),
			(&StateEventType::RoomJoinRules, ""),
			(&StateEventType::RoomCanonicalAlias, ""),
			(&StateEventType::RoomName, ""),
			(&StateEventType::RoomAvatar, ""),
			(&StateEventType::RoomMember, invite.sender.as_str()), // Add recommended events
		];

		let fetches = cells.iter().map(|(event_type, state_key)| {
			self.services
				.state_accessor
				.room_state_get(&invite.room_id, event_type, state_key)
		});

		join_all(fetches)
			.await
			.into_iter()
			.filter_map(Result::ok)
			.map(|e| e.to_stripped_state_event())
			.chain(once(invite.to_stripped_state_event()))
			.collect()
	}

	/// Set the state hash to a new version, but does not update state_cache.
	#[tracing::instrument(skip(self, _mutex_lock), level = "debug")]
	pub fn set_room_state(
		&self,
		room_id: &RoomId,
		shortstatehash: u64,
		_mutex_lock: &RoomMutexGuard, // Take mutex guard to make sure users get the room state mutex
	) {
		const BUFSIZE: usize = size_of::<u64>();

		self.db
			.roomid_shortstatehash
			.raw_aput::<BUFSIZE, _, _>(room_id, shortstatehash);
	}

	/// Returns the room's version.
	#[tracing::instrument(skip(self), level = "debug")]
	pub async fn get_room_version(&self, room_id: &RoomId) -> Result<RoomVersionId> {
		self.services
			.state_accessor
			.room_state_get_content(room_id, &StateEventType::RoomCreate, "")
			.await
			.map(|content: RoomCreateEventContent| content.room_version)
			.map_err(|e| err!(Request(NotFound("No create event found: {e:?}"))))
	}

	pub async fn get_room_shortstatehash(&self, room_id: &RoomId) -> Result<u64> {
		self.db
			.roomid_shortstatehash
			.get(room_id)
			.await
			.deserialized()
	}

	pub fn get_forward_extremities<'a>(&'a self, room_id: &'a RoomId) -> impl Stream<Item = &EventId> + Send + '_ {
		let prefix = (room_id, Interfix);

		self.db
			.roomid_pduleaves
			.keys_prefix(&prefix)
			.map_ok(|(_, event_id): (Ignore, &EventId)| event_id)
			.ignore_err()
	}

	pub async fn set_forward_extremities(
		&self,
		room_id: &RoomId,
		event_ids: Vec<OwnedEventId>,
		_state_lock: &RoomMutexGuard, // Take mutex guard to make sure users get the room state mutex
	) {
		let prefix = (room_id, Interfix);
		self.db
			.roomid_pduleaves
			.keys_prefix_raw(&prefix)
			.ignore_err()
			.ready_for_each(|key| self.db.roomid_pduleaves.remove(key))
			.await;

		for event_id in &event_ids {
			let key = (room_id, event_id);
			self.db.roomid_pduleaves.put_raw(key, event_id);
		}
	}

	/// This fetches auth events from the current state.
	#[tracing::instrument(skip(self), level = "debug")]
	pub async fn get_auth_events(
		&self, room_id: &RoomId, kind: &TimelineEventType, sender: &UserId, state_key: Option<&str>,
		content: &serde_json::value::RawValue,
	) -> Result<StateMap<Arc<PduEvent>>> {
		let Ok(shortstatehash) = self.get_room_shortstatehash(room_id).await else {
			return Ok(HashMap::new());
		};

		let auth_events = state_res::auth_types_for_event(kind, sender, state_key, content)?;

		let mut sauthevents: HashMap<_, _> = auth_events
			.iter()
			.stream()
			.filter_map(|(event_type, state_key)| {
				self.services
					.short
					.get_shortstatekey(event_type, state_key)
					.map_ok(move |s| (s, (event_type, state_key)))
					.map(Result::ok)
			})
			.collect()
			.await;

		let full_state = self
			.services
			.state_compressor
			.load_shortstatehash_info(shortstatehash)
			.await
			.map_err(|e| {
				err!(Database(
					"Missing shortstatehash info for {room_id:?} at {shortstatehash:?}: {e:?}"
				))
			})?
			.pop()
			.expect("there is always one layer")
			.1;

		let mut ret = HashMap::new();
		for compressed in full_state.iter() {
			let Ok((shortstatekey, event_id)) = self
				.services
				.state_compressor
				.parse_compressed_state_event(compressed)
				.await
			else {
				continue;
			};

			let Some((ty, state_key)) = sauthevents.remove(&shortstatekey) else {
				continue;
			};

			let Ok(pdu) = self.services.timeline.get_pdu(&event_id).await else {
				continue;
			};

			ret.insert((ty.to_owned(), state_key.to_owned()), pdu);
		}

		Ok(ret)
	}
}
