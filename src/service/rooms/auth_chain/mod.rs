mod data;

use std::{
	collections::{BTreeSet, HashSet},
	sync::Arc,
};

use conduit::{debug, debug_error, trace, utils::IterStream, validated, warn, Err, Result};
use futures::{FutureExt, Stream, StreamExt};
use ruma::{EventId, RoomId};

use self::data::Data;
use crate::{rooms, Dep};

pub struct Service {
	services: Services,
	db: Data,
}

struct Services {
	short: Dep<rooms::short::Service>,
	timeline: Dep<rooms::timeline::Service>,
}

impl crate::Service for Service {
	fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
		Ok(Arc::new(Self {
			services: Services {
				short: args.depend::<rooms::short::Service>("rooms::short"),
				timeline: args.depend::<rooms::timeline::Service>("rooms::timeline"),
			},
			db: Data::new(&args),
		}))
	}

	fn name(&self) -> &str { crate::service::make_name(std::module_path!()) }
}

impl Service {
	pub async fn event_ids_iter(
		&self, room_id: &RoomId, starting_events: &[&EventId],
	) -> Result<impl Stream<Item = Arc<EventId>> + Send + '_> {
		let chain = self.get_auth_chain(room_id, starting_events).await?;
		let iter = chain.into_iter().stream().filter_map(|sid| {
			self.services
				.short
				.get_eventid_from_short(sid)
				.map(Result::ok)
		});

		Ok(iter)
	}

	#[tracing::instrument(skip_all, name = "auth_chain")]
	pub async fn get_auth_chain(&self, room_id: &RoomId, starting_events: &[&EventId]) -> Result<Vec<u64>> {
		const NUM_BUCKETS: usize = 50; //TODO: change possible w/o disrupting db?
		const BUCKET: BTreeSet<(u64, &EventId)> = BTreeSet::new();

		let started = std::time::Instant::now();
		let mut buckets = [BUCKET; NUM_BUCKETS];
		for (i, &short) in self
			.services
			.short
			.multi_get_or_create_shorteventid(starting_events)
			.await
			.iter()
			.enumerate()
		{
			let bucket: usize = short.try_into()?;
			let bucket: usize = validated!(bucket % NUM_BUCKETS);
			buckets[bucket].insert((short, starting_events[i]));
		}

		debug!(
			starting_events = ?starting_events.len(),
			elapsed = ?started.elapsed(),
			"start",
		);

		let mut hits: usize = 0;
		let mut misses: usize = 0;
		let mut full_auth_chain = Vec::with_capacity(buckets.len());
		for chunk in buckets {
			if chunk.is_empty() {
				continue;
			}

			let chunk_key: Vec<u64> = chunk.iter().map(|(short, _)| short).copied().collect();
			if let Ok(cached) = self.get_cached_eventid_authchain(&chunk_key).await {
				trace!("Found cache entry for whole chunk");
				full_auth_chain.extend(cached.iter().copied());
				hits = hits.saturating_add(1);
				continue;
			}

			let mut hits2: usize = 0;
			let mut misses2: usize = 0;
			let mut chunk_cache = Vec::with_capacity(chunk.len());
			for (sevent_id, event_id) in chunk {
				if let Ok(cached) = self.get_cached_eventid_authchain(&[sevent_id]).await {
					trace!(?event_id, "Found cache entry for event");
					chunk_cache.extend(cached.iter().copied());
					hits2 = hits2.saturating_add(1);
				} else {
					let auth_chain = self.get_auth_chain_inner(room_id, event_id).await?;
					self.cache_auth_chain(vec![sevent_id], &auth_chain);
					chunk_cache.extend(auth_chain.iter());
					misses2 = misses2.saturating_add(1);
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
			self.cache_auth_chain_vec(chunk_key, &chunk_cache);
			full_auth_chain.extend(chunk_cache.iter());
			misses = misses.saturating_add(1);
			debug!(
				chunk_cache_length = ?chunk_cache.len(),
				hits = ?hits2,
				misses = ?misses2,
				elapsed = ?started.elapsed(),
				"Chunk missed",
			);
		}

		full_auth_chain.sort_unstable();
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

	#[tracing::instrument(skip(self, room_id))]
	async fn get_auth_chain_inner(&self, room_id: &RoomId, event_id: &EventId) -> Result<HashSet<u64>> {
		let mut todo = vec![Arc::from(event_id)];
		let mut found = HashSet::new();

		while let Some(event_id) = todo.pop() {
			trace!(?event_id, "processing auth event");

			match self.services.timeline.get_pdu(&event_id).await {
				Err(e) => debug_error!(?event_id, ?e, "Could not find pdu mentioned in auth events"),
				Ok(pdu) => {
					if pdu.room_id != room_id {
						return Err!(Request(Forbidden(
							"auth event {event_id:?} for incorrect room {} which is not {room_id}",
							pdu.room_id,
						)));
					}

					for auth_event in &pdu.auth_events {
						let sauthevent = self
							.services
							.short
							.get_or_create_shorteventid(auth_event)
							.await;

						if found.insert(sauthevent) {
							trace!(?event_id, ?auth_event, "adding auth event to processing queue");
							todo.push(auth_event.clone());
						}
					}
				},
			}
		}

		Ok(found)
	}

	#[inline]
	pub async fn get_cached_eventid_authchain(&self, key: &[u64]) -> Result<Arc<[u64]>> {
		self.db.get_cached_eventid_authchain(key).await
	}

	#[tracing::instrument(skip(self), level = "debug")]
	pub fn cache_auth_chain(&self, key: Vec<u64>, auth_chain: &HashSet<u64>) {
		let val = auth_chain.iter().copied().collect::<Arc<[u64]>>();
		self.db.cache_auth_chain(key, val);
	}

	#[tracing::instrument(skip(self), level = "debug")]
	pub fn cache_auth_chain_vec(&self, key: Vec<u64>, auth_chain: &Vec<u64>) {
		let val = auth_chain.iter().copied().collect::<Arc<[u64]>>();
		self.db.cache_auth_chain(key, val);
	}

	pub fn get_cache_usage(&self) -> (usize, usize) {
		let cache = self.db.auth_chain_cache.lock().expect("locked");
		(cache.len(), cache.capacity())
	}

	pub fn clear_cache(&self) { self.db.auth_chain_cache.lock().expect("locked").clear(); }
}
