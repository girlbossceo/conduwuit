mod data;
use std::{
	collections::{BTreeSet, HashSet},
	sync::Arc,
};

pub(crate) use data::Data;
use ruma::{api::client::error::ErrorKind, EventId, RoomId};
use tracing::{debug, error, warn};

use crate::{services, Error, Result};

pub(crate) struct Service {
	pub(crate) db: &'static dyn Data,
}

impl Service {
	pub(crate) async fn event_ids_iter<'a>(
		&self, room_id: &RoomId, starting_events_: Vec<Arc<EventId>>,
	) -> Result<impl Iterator<Item = Arc<EventId>> + 'a> {
		let mut starting_events: Vec<&EventId> = Vec::with_capacity(starting_events_.len());
		for starting_event in &starting_events_ {
			starting_events.push(starting_event);
		}

		Ok(self
			.get_auth_chain(room_id, &starting_events)
			.await?
			.into_iter()
			.filter_map(move |sid| services().rooms.short.get_eventid_from_short(sid).ok()))
	}

	pub(crate) async fn get_auth_chain(&self, room_id: &RoomId, starting_events: &[&EventId]) -> Result<Vec<u64>> {
		const NUM_BUCKETS: usize = 50; //TODO: change possible w/o disrupting db?
		const BUCKET: BTreeSet<(u64, &EventId)> = BTreeSet::new();

		let started = std::time::Instant::now();
		let mut buckets = [BUCKET; NUM_BUCKETS];
		for (i, short) in services()
			.rooms
			.short
			.multi_get_or_create_shorteventid(starting_events)?
			.iter()
			.enumerate()
		{
			let bucket = short % NUM_BUCKETS as u64;
			buckets[bucket as usize].insert((*short, starting_events[i]));
		}

		debug!(
			starting_events = ?starting_events.len(),
			elapsed = ?started.elapsed(),
			"start",
		);

		let mut hits = 0;
		let mut misses = 0;
		let mut full_auth_chain = Vec::new();
		for chunk in buckets {
			if chunk.is_empty() {
				continue;
			}

			let chunk_key: Vec<u64> = chunk.iter().map(|(short, _)| short).copied().collect();
			if let Some(cached) = services()
				.rooms
				.auth_chain
				.get_cached_eventid_authchain(&chunk_key)?
			{
				full_auth_chain.extend(cached.iter().copied());
				hits += 1;
				continue;
			}

			let mut hits2 = 0;
			let mut misses2 = 0;
			let mut chunk_cache = Vec::new();
			for (sevent_id, event_id) in chunk {
				if let Some(cached) = services()
					.rooms
					.auth_chain
					.get_cached_eventid_authchain(&[sevent_id])?
				{
					chunk_cache.extend(cached.iter().copied());
					hits2 += 1;
				} else {
					let auth_chain = self.get_auth_chain_inner(room_id, event_id)?;
					services()
						.rooms
						.auth_chain
						.cache_auth_chain(vec![sevent_id], &auth_chain)?;
					chunk_cache.extend(auth_chain.iter());
					misses2 += 1;
					debug!(
						event_id = ?event_id,
						chain_length = ?auth_chain.len(),
						chunk_cache_length = ?chunk_cache.len(),
						elapsed = ?started.elapsed(),
						"Cache missed event"
					);
				};
			}

			chunk_cache.sort_unstable();
			chunk_cache.dedup();
			services()
				.rooms
				.auth_chain
				.cache_auth_chain_vec(chunk_key, &chunk_cache)?;
			full_auth_chain.extend(chunk_cache.iter());
			misses += 1;
			debug!(
				chunk_cache_length = ?chunk_cache.len(),
				hits = ?hits2,
				misses = ?misses2,
				elapsed = ?started.elapsed(),
				"Chunk missed",
			);
		}

		full_auth_chain.sort();
		full_auth_chain.dedup();
		debug!(
			chain_length = ?full_auth_chain.len(),
			hits = ?hits,
			misses = ?misses,
			elapsed = ?started.elapsed(),
			"done",
		);

		Ok(full_auth_chain)
	}

	#[tracing::instrument(skip(self, event_id))]
	fn get_auth_chain_inner(&self, room_id: &RoomId, event_id: &EventId) -> Result<HashSet<u64>> {
		let mut todo = vec![Arc::from(event_id)];
		let mut found = HashSet::new();

		while let Some(event_id) = todo.pop() {
			match services().rooms.timeline.get_pdu(&event_id) {
				Ok(Some(pdu)) => {
					if pdu.room_id != room_id {
						return Err(Error::BadRequest(ErrorKind::forbidden(), "Evil event in db"));
					}
					for auth_event in &pdu.auth_events {
						let sauthevent = services()
							.rooms
							.short
							.get_or_create_shorteventid(auth_event)?;

						if found.insert(sauthevent) {
							todo.push(auth_event.clone());
						}
					}
				},
				Ok(None) => {
					warn!(?event_id, "Could not find pdu mentioned in auth events");
				},
				Err(error) => {
					error!(?event_id, ?error, "Could not load event in auth chain");
				},
			}
		}

		Ok(found)
	}

	pub(crate) fn get_cached_eventid_authchain(&self, key: &[u64]) -> Result<Option<Arc<[u64]>>> {
		self.db.get_cached_eventid_authchain(key)
	}

	#[tracing::instrument(skip(self))]
	pub(crate) fn cache_auth_chain(&self, key: Vec<u64>, auth_chain: &HashSet<u64>) -> Result<()> {
		self.db
			.cache_auth_chain(key, auth_chain.iter().copied().collect::<Arc<[u64]>>())
	}

	#[tracing::instrument(skip(self))]
	pub(crate) fn cache_auth_chain_vec(&self, key: Vec<u64>, auth_chain: &Vec<u64>) -> Result<()> {
		self.db
			.cache_auth_chain(key, auth_chain.iter().copied().collect::<Arc<[u64]>>())
	}
}
