use std::{
	collections::HashSet,
	fmt::Write,
	mem::size_of,
	sync::{Arc, Mutex},
};

use conduit::{checked, err, expected, utils, utils::math::usize_from_f64, Result};
use database::Map;
use lru_cache::LruCache;
use ruma::{EventId, RoomId};

use crate::{rooms, Dep};

pub struct Service {
	pub stateinfo_cache: Mutex<StateInfoLruCache>,
	db: Data,
	services: Services,
}

struct Services {
	short: Dep<rooms::short::Service>,
	state: Dep<rooms::state::Service>,
}

struct Data {
	shortstatehash_statediff: Arc<Map>,
}

struct StateDiff {
	parent: Option<u64>,
	added: Arc<HashSet<CompressedStateEvent>>,
	removed: Arc<HashSet<CompressedStateEvent>>,
}

type StateInfoLruCache = LruCache<u64, ShortStateInfoVec>;
type ShortStateInfoVec = Vec<ShortStateInfo>;
type ParentStatesVec = Vec<ShortStateInfo>;
type ShortStateInfo = (
	u64,                                // sstatehash
	Arc<HashSet<CompressedStateEvent>>, // full state
	Arc<HashSet<CompressedStateEvent>>, // added
	Arc<HashSet<CompressedStateEvent>>, // removed
);

type HashSetCompressStateEvent = (u64, Arc<HashSet<CompressedStateEvent>>, Arc<HashSet<CompressedStateEvent>>);
pub type CompressedStateEvent = [u8; 2 * size_of::<u64>()];

impl crate::Service for Service {
	fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
		let config = &args.server.config;
		let cache_capacity = f64::from(config.stateinfo_cache_capacity) * config.cache_capacity_modifier;
		Ok(Arc::new(Self {
			stateinfo_cache: LruCache::new(usize_from_f64(cache_capacity)?).into(),
			db: Data {
				shortstatehash_statediff: args.db["shortstatehash_statediff"].clone(),
			},
			services: Services {
				short: args.depend::<rooms::short::Service>("rooms::short"),
				state: args.depend::<rooms::state::Service>("rooms::state"),
			},
		}))
	}

	fn memory_usage(&self, out: &mut dyn Write) -> Result<()> {
		let stateinfo_cache = self.stateinfo_cache.lock().expect("locked").len();
		writeln!(out, "stateinfo_cache: {stateinfo_cache}")?;

		Ok(())
	}

	fn clear_cache(&self) { self.stateinfo_cache.lock().expect("locked").clear(); }

	fn name(&self) -> &str { crate::service::make_name(std::module_path!()) }
}

impl Service {
	/// Returns a stack with info on shortstatehash, full state, added diff and
	/// removed diff for the selected shortstatehash and each parent layer.
	pub async fn load_shortstatehash_info(&self, shortstatehash: u64) -> Result<ShortStateInfoVec> {
		if let Some(r) = self
			.stateinfo_cache
			.lock()
			.expect("locked")
			.get_mut(&shortstatehash)
		{
			return Ok(r.clone());
		}

		let StateDiff {
			parent,
			added,
			removed,
		} = self.get_statediff(shortstatehash).await?;

		if let Some(parent) = parent {
			let mut response = Box::pin(self.load_shortstatehash_info(parent)).await?;
			let mut state = (*response.last().expect("at least one response").1).clone();
			state.extend(added.iter().copied());
			let removed = (*removed).clone();
			for r in &removed {
				state.remove(r);
			}

			response.push((shortstatehash, Arc::new(state), added, Arc::new(removed)));

			self.stateinfo_cache
				.lock()
				.expect("locked")
				.insert(shortstatehash, response.clone());

			Ok(response)
		} else {
			let response = vec![(shortstatehash, added.clone(), added, removed)];
			self.stateinfo_cache
				.lock()
				.expect("locked")
				.insert(shortstatehash, response.clone());

			Ok(response)
		}
	}

	pub async fn compress_state_event(&self, shortstatekey: u64, event_id: &EventId) -> CompressedStateEvent {
		let mut v = shortstatekey.to_be_bytes().to_vec();
		v.extend_from_slice(
			&self
				.services
				.short
				.get_or_create_shorteventid(event_id)
				.await
				.to_be_bytes(),
		);

		v.try_into().expect("we checked the size above")
	}

	/// Returns shortstatekey, event id
	#[inline]
	pub async fn parse_compressed_state_event(
		&self, compressed_event: &CompressedStateEvent,
	) -> Result<(u64, Arc<EventId>)> {
		use utils::u64_from_u8;

		let shortstatekey = u64_from_u8(&compressed_event[0..size_of::<u64>()]);
		let event_id = self
			.services
			.short
			.get_eventid_from_short(u64_from_u8(&compressed_event[size_of::<u64>()..]))
			.await?;

		Ok((shortstatekey, event_id))
	}

	/// Creates a new shortstatehash that often is just a diff to an already
	/// existing shortstatehash and therefore very efficient.
	///
	/// There are multiple layers of diffs. The bottom layer 0 always contains
	/// the full state. Layer 1 contains diffs to states of layer 0, layer 2
	/// diffs to layer 1 and so on. If layer n > 0 grows too big, it will be
	/// combined with layer n-1 to create a new diff on layer n-1 that's
	/// based on layer n-2. If that layer is also too big, it will recursively
	/// fix above layers too.
	///
	/// * `shortstatehash` - Shortstatehash of this state
	/// * `statediffnew` - Added to base. Each vec is shortstatekey+shorteventid
	/// * `statediffremoved` - Removed from base. Each vec is
	///   shortstatekey+shorteventid
	/// * `diff_to_sibling` - Approximately how much the diff grows each time
	///   for this layer
	/// * `parent_states` - A stack with info on shortstatehash, full state,
	///   added diff and removed diff for each parent layer
	#[tracing::instrument(skip_all, level = "debug")]
	pub fn save_state_from_diff(
		&self, shortstatehash: u64, statediffnew: Arc<HashSet<CompressedStateEvent>>,
		statediffremoved: Arc<HashSet<CompressedStateEvent>>, diff_to_sibling: usize,
		mut parent_states: ParentStatesVec,
	) -> Result {
		let statediffnew_len = statediffnew.len();
		let statediffremoved_len = statediffremoved.len();
		let diffsum = checked!(statediffnew_len + statediffremoved_len)?;

		if parent_states.len() > 3 {
			// Number of layers
			// To many layers, we have to go deeper
			let parent = parent_states.pop().expect("parent must have a state");

			let mut parent_new = (*parent.2).clone();
			let mut parent_removed = (*parent.3).clone();

			for removed in statediffremoved.iter() {
				if !parent_new.remove(removed) {
					// It was not added in the parent and we removed it
					parent_removed.insert(*removed);
				}
				// Else it was added in the parent and we removed it again. We
				// can forget this change
			}

			for new in statediffnew.iter() {
				if !parent_removed.remove(new) {
					// It was not touched in the parent and we added it
					parent_new.insert(*new);
				}
				// Else it was removed in the parent and we added it again. We
				// can forget this change
			}

			self.save_state_from_diff(
				shortstatehash,
				Arc::new(parent_new),
				Arc::new(parent_removed),
				diffsum,
				parent_states,
			)?;

			return Ok(());
		}

		if parent_states.is_empty() {
			// There is no parent layer, create a new state
			self.save_statediff(
				shortstatehash,
				&StateDiff {
					parent: None,
					added: statediffnew,
					removed: statediffremoved,
				},
			);

			return Ok(());
		};

		// Else we have two options.
		// 1. We add the current diff on top of the parent layer.
		// 2. We replace a layer above

		let parent = parent_states.pop().expect("parent must have a state");
		let parent_2_len = parent.2.len();
		let parent_3_len = parent.3.len();
		let parent_diff = checked!(parent_2_len + parent_3_len)?;

		if checked!(diffsum * diffsum)? >= checked!(2 * diff_to_sibling * parent_diff)? {
			// Diff too big, we replace above layer(s)
			let mut parent_new = (*parent.2).clone();
			let mut parent_removed = (*parent.3).clone();

			for removed in statediffremoved.iter() {
				if !parent_new.remove(removed) {
					// It was not added in the parent and we removed it
					parent_removed.insert(*removed);
				}
				// Else it was added in the parent and we removed it again. We
				// can forget this change
			}

			for new in statediffnew.iter() {
				if !parent_removed.remove(new) {
					// It was not touched in the parent and we added it
					parent_new.insert(*new);
				}
				// Else it was removed in the parent and we added it again. We
				// can forget this change
			}

			self.save_state_from_diff(
				shortstatehash,
				Arc::new(parent_new),
				Arc::new(parent_removed),
				diffsum,
				parent_states,
			)?;
		} else {
			// Diff small enough, we add diff as layer on top of parent
			self.save_statediff(
				shortstatehash,
				&StateDiff {
					parent: Some(parent.0),
					added: statediffnew,
					removed: statediffremoved,
				},
			);
		}

		Ok(())
	}

	/// Returns the new shortstatehash, and the state diff from the previous
	/// room state
	pub async fn save_state(
		&self, room_id: &RoomId, new_state_ids_compressed: Arc<HashSet<CompressedStateEvent>>,
	) -> Result<HashSetCompressStateEvent> {
		let previous_shortstatehash = self
			.services
			.state
			.get_room_shortstatehash(room_id)
			.await
			.ok();

		let state_hash = utils::calculate_hash(
			&new_state_ids_compressed
				.iter()
				.map(|bytes| &bytes[..])
				.collect::<Vec<_>>(),
		);

		let (new_shortstatehash, already_existed) = self
			.services
			.short
			.get_or_create_shortstatehash(&state_hash)
			.await;

		if Some(new_shortstatehash) == previous_shortstatehash {
			return Ok((new_shortstatehash, Arc::new(HashSet::new()), Arc::new(HashSet::new())));
		}

		let states_parents = if let Some(p) = previous_shortstatehash {
			self.load_shortstatehash_info(p).await.unwrap_or_default()
		} else {
			ShortStateInfoVec::new()
		};

		let (statediffnew, statediffremoved) = if let Some(parent_stateinfo) = states_parents.last() {
			let statediffnew: HashSet<_> = new_state_ids_compressed
				.difference(&parent_stateinfo.1)
				.copied()
				.collect();

			let statediffremoved: HashSet<_> = parent_stateinfo
				.1
				.difference(&new_state_ids_compressed)
				.copied()
				.collect();

			(Arc::new(statediffnew), Arc::new(statediffremoved))
		} else {
			(new_state_ids_compressed, Arc::new(HashSet::new()))
		};

		if !already_existed {
			self.save_state_from_diff(
				new_shortstatehash,
				statediffnew.clone(),
				statediffremoved.clone(),
				2, // every state change is 2 event changes on average
				states_parents,
			)?;
		};

		Ok((new_shortstatehash, statediffnew, statediffremoved))
	}

	async fn get_statediff(&self, shortstatehash: u64) -> Result<StateDiff> {
		const BUFSIZE: usize = size_of::<u64>();
		const STRIDE: usize = size_of::<u64>();

		let value = self
			.db
			.shortstatehash_statediff
			.aqry::<BUFSIZE, _>(&shortstatehash)
			.await
			.map_err(|e| err!(Database("Failed to find StateDiff from short {shortstatehash:?}: {e}")))?;

		let parent = utils::u64_from_bytes(&value[0..size_of::<u64>()])
			.ok()
			.take_if(|parent| *parent != 0);

		let mut add_mode = true;
		let mut added = HashSet::new();
		let mut removed = HashSet::new();

		let mut i = STRIDE;
		while let Some(v) = value.get(i..expected!(i + 2 * STRIDE)) {
			if add_mode && v.starts_with(&0_u64.to_be_bytes()) {
				add_mode = false;
				i = expected!(i + STRIDE);
				continue;
			}
			if add_mode {
				added.insert(v.try_into()?);
			} else {
				removed.insert(v.try_into()?);
			}
			i = expected!(i + 2 * STRIDE);
		}

		Ok(StateDiff {
			parent,
			added: Arc::new(added),
			removed: Arc::new(removed),
		})
	}

	fn save_statediff(&self, shortstatehash: u64, diff: &StateDiff) {
		let mut value = diff.parent.unwrap_or(0).to_be_bytes().to_vec();
		for new in diff.added.iter() {
			value.extend_from_slice(&new[..]);
		}

		if !diff.removed.is_empty() {
			value.extend_from_slice(&0_u64.to_be_bytes());
			for removed in diff.removed.iter() {
				value.extend_from_slice(&removed[..]);
			}
		}

		self.db
			.shortstatehash_statediff
			.insert(&shortstatehash.to_be_bytes(), &value);
	}
}
