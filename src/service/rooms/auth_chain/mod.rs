mod data;

use std::{
	collections::{BTreeSet, HashSet, VecDeque},
	fmt::Debug,
	sync::Arc,
};

use conduwuit::{
	at, debug, debug_error, trace,
	utils::{
		stream::{ReadyExt, TryBroadbandExt},
		IterStream,
	},
	validated, warn, Err, Result,
};
use futures::{Stream, StreamExt, TryStreamExt};
use ruma::{EventId, OwnedEventId, RoomId};

use self::data::Data;
use crate::{rooms, rooms::short::ShortEventId, Dep};

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
	pub async fn event_ids_iter<'a, I>(
		&'a self,
		room_id: &RoomId,
		starting_events: I,
	) -> Result<impl Stream<Item = OwnedEventId> + Send + '_>
	where
		I: Iterator<Item = &'a EventId> + Clone + Debug + ExactSizeIterator + Send + 'a,
	{
		let stream = self
			.get_event_ids(room_id, starting_events)
			.await?
			.into_iter()
			.stream();

		Ok(stream)
	}

	pub async fn get_event_ids<'a, I>(
		&'a self,
		room_id: &RoomId,
		starting_events: I,
	) -> Result<Vec<OwnedEventId>>
	where
		I: Iterator<Item = &'a EventId> + Clone + Debug + ExactSizeIterator + Send + 'a,
	{
		let chain = self.get_auth_chain(room_id, starting_events).await?;
		let event_ids = self
			.services
			.short
			.multi_get_eventid_from_short(chain.into_iter().stream())
			.ready_filter_map(Result::ok)
			.collect()
			.await;

		Ok(event_ids)
	}

	#[tracing::instrument(name = "auth_chain", level = "debug", skip_all)]
	pub async fn get_auth_chain<'a, I>(
		&'a self,
		room_id: &RoomId,
		starting_events: I,
	) -> Result<Vec<ShortEventId>>
	where
		I: Iterator<Item = &'a EventId> + Clone + Debug + ExactSizeIterator + Send + 'a,
	{
		const NUM_BUCKETS: usize = 50; //TODO: change possible w/o disrupting db?
		const BUCKET: BTreeSet<(u64, &EventId)> = BTreeSet::new();

		let started = std::time::Instant::now();
		let mut starting_ids = self
			.services
			.short
			.multi_get_or_create_shorteventid(starting_events.clone())
			.zip(starting_events.clone().stream())
			.boxed();

		let mut buckets = [BUCKET; NUM_BUCKETS];
		while let Some((short, starting_event)) = starting_ids.next().await {
			let bucket: usize = short.try_into()?;
			let bucket: usize = validated!(bucket % NUM_BUCKETS);
			buckets[bucket].insert((short, starting_event));
		}

		debug!(
			starting_events = ?starting_events.count(),
			elapsed = ?started.elapsed(),
			"start",
		);

		let full_auth_chain: Vec<_> = buckets
			.into_iter()
			.try_stream()
			.broad_and_then(|chunk| async move {
				let chunk_key: Vec<ShortEventId> = chunk.iter().map(at!(0)).collect();

				if chunk_key.is_empty() {
					return Ok(Vec::new());
				}

				if let Ok(cached) = self.get_cached_eventid_authchain(&chunk_key).await {
					return Ok(cached.to_vec());
				}

				let chunk_cache: Vec<_> = chunk
					.into_iter()
					.try_stream()
					.broad_and_then(|(shortid, event_id)| async move {
						if let Ok(cached) = self.get_cached_eventid_authchain(&[shortid]).await {
							return Ok(cached.to_vec());
						}

						let auth_chain = self.get_auth_chain_inner(room_id, event_id).await?;
						self.cache_auth_chain_vec(vec![shortid], auth_chain.as_slice());
						debug!(
							?event_id,
							elapsed = ?started.elapsed(),
							"Cache missed event"
						);

						Ok(auth_chain)
					})
					.try_collect()
					.await?;

				let mut chunk_cache: Vec<_> = chunk_cache.into_iter().flatten().collect();
				chunk_cache.sort_unstable();
				chunk_cache.dedup();
				self.cache_auth_chain_vec(chunk_key, chunk_cache.as_slice());
				debug!(
					chunk_cache_length = ?chunk_cache.len(),
					elapsed = ?started.elapsed(),
					"Cache missed chunk",
				);

				Ok(chunk_cache)
			})
			.try_collect()
			.await?;

		let mut full_auth_chain: Vec<_> = full_auth_chain.into_iter().flatten().collect();
		full_auth_chain.sort_unstable();
		full_auth_chain.dedup();
		debug!(
			chain_length = ?full_auth_chain.len(),
			elapsed = ?started.elapsed(),
			"done",
		);

		Ok(full_auth_chain)
	}

	#[tracing::instrument(name = "inner", level = "trace", skip(self, room_id))]
	async fn get_auth_chain_inner(
		&self,
		room_id: &RoomId,
		event_id: &EventId,
	) -> Result<Vec<ShortEventId>> {
		let mut todo: VecDeque<_> = [event_id.to_owned()].into();
		let mut found = HashSet::new();

		while let Some(event_id) = todo.pop_front() {
			trace!(?event_id, "processing auth event");

			match self.services.timeline.get_pdu(&event_id).await {
				| Err(e) => {
					debug_error!(?event_id, ?e, "Could not find pdu mentioned in auth events");
				},
				| Ok(pdu) => {
					if pdu.room_id != room_id {
						return Err!(Request(Forbidden(error!(
							?event_id,
							?room_id,
							wrong_room_id = ?pdu.room_id,
							"auth event for incorrect room"
						))));
					}

					for auth_event in &pdu.auth_events {
						let sauthevent = self
							.services
							.short
							.get_or_create_shorteventid(auth_event)
							.await;

						if found.insert(sauthevent) {
							trace!(
								?event_id,
								?auth_event,
								"adding auth event to processing queue"
							);

							todo.push_back(auth_event.clone());
						}
					}
				},
			}
		}

		Ok(found.into_iter().collect())
	}

	#[inline]
	pub async fn get_cached_eventid_authchain(&self, key: &[u64]) -> Result<Arc<[ShortEventId]>> {
		self.db.get_cached_eventid_authchain(key).await
	}

	#[tracing::instrument(skip_all, level = "debug")]
	pub fn cache_auth_chain(&self, key: Vec<u64>, auth_chain: &HashSet<ShortEventId>) {
		let val: Arc<[ShortEventId]> = auth_chain.iter().copied().collect();

		self.db.cache_auth_chain(key, val);
	}

	#[tracing::instrument(skip_all, level = "debug")]
	pub fn cache_auth_chain_vec(&self, key: Vec<u64>, auth_chain: &[ShortEventId]) {
		let val: Arc<[ShortEventId]> = auth_chain.iter().copied().collect();

		self.db.cache_auth_chain(key, val);
	}

	pub fn get_cache_usage(&self) -> (usize, usize) {
		let cache = self.db.auth_chain_cache.lock().expect("locked");

		(cache.len(), cache.capacity())
	}

	pub fn clear_cache(&self) { self.db.auth_chain_cache.lock().expect("locked").clear(); }
}
