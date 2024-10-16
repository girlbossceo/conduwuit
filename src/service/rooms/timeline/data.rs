use std::{
	collections::{hash_map, HashMap},
	mem::size_of,
	sync::Arc,
};

use conduit::{
	err, expected,
	result::{LogErr, NotFound},
	utils,
	utils::{future::TryExtExt, stream::TryIgnore, u64_from_u8, ReadyExt},
	Err, PduCount, PduEvent, Result,
};
use database::{Database, Deserialized, Json, KeyVal, Map};
use futures::{Stream, StreamExt};
use ruma::{CanonicalJsonObject, EventId, OwnedRoomId, OwnedUserId, RoomId, UserId};
use tokio::sync::Mutex;

use crate::{rooms, Dep};

pub(super) struct Data {
	eventid_outlierpdu: Arc<Map>,
	eventid_pduid: Arc<Map>,
	pduid_pdu: Arc<Map>,
	userroomid_highlightcount: Arc<Map>,
	userroomid_notificationcount: Arc<Map>,
	pub(super) lasttimelinecount_cache: LastTimelineCountCache,
	pub(super) db: Arc<Database>,
	services: Services,
}

struct Services {
	short: Dep<rooms::short::Service>,
}

pub type PdusIterItem = (PduCount, PduEvent);
type LastTimelineCountCache = Mutex<HashMap<OwnedRoomId, PduCount>>;

impl Data {
	pub(super) fn new(args: &crate::Args<'_>) -> Self {
		let db = &args.db;
		Self {
			eventid_outlierpdu: db["eventid_outlierpdu"].clone(),
			eventid_pduid: db["eventid_pduid"].clone(),
			pduid_pdu: db["pduid_pdu"].clone(),
			userroomid_highlightcount: db["userroomid_highlightcount"].clone(),
			userroomid_notificationcount: db["userroomid_notificationcount"].clone(),
			lasttimelinecount_cache: Mutex::new(HashMap::new()),
			db: args.db.clone(),
			services: Services {
				short: args.depend::<rooms::short::Service>("rooms::short"),
			},
		}
	}

	pub(super) async fn last_timeline_count(&self, sender_user: &UserId, room_id: &RoomId) -> Result<PduCount> {
		match self
			.lasttimelinecount_cache
			.lock()
			.await
			.entry(room_id.to_owned())
		{
			hash_map::Entry::Vacant(v) => {
				if let Some(last_count) = self
					.pdus_until(sender_user, room_id, PduCount::max())
					.await?
					.next()
					.await
				{
					Ok(*v.insert(last_count.0))
				} else {
					Ok(PduCount::Normal(0))
				}
			},
			hash_map::Entry::Occupied(o) => Ok(*o.get()),
		}
	}

	/// Returns the `count` of this pdu's id.
	pub(super) async fn get_pdu_count(&self, event_id: &EventId) -> Result<PduCount> {
		self.eventid_pduid
			.get(event_id)
			.await
			.map(|pdu_id| pdu_count(&pdu_id))
	}

	/// Returns the json of a pdu.
	pub(super) async fn get_pdu_json(&self, event_id: &EventId) -> Result<CanonicalJsonObject> {
		if let Ok(pdu) = self.get_non_outlier_pdu_json(event_id).await {
			return Ok(pdu);
		}

		self.eventid_outlierpdu.get(event_id).await.deserialized()
	}

	/// Returns the json of a pdu.
	pub(super) async fn get_non_outlier_pdu_json(&self, event_id: &EventId) -> Result<CanonicalJsonObject> {
		let pduid = self.get_pdu_id(event_id).await?;

		self.pduid_pdu.get(&pduid).await.deserialized()
	}

	/// Returns the pdu's id.
	#[inline]
	pub(super) async fn get_pdu_id(&self, event_id: &EventId) -> Result<database::Handle<'_>> {
		self.eventid_pduid.get(event_id).await
	}

	/// Returns the pdu directly from `eventid_pduid` only.
	pub(super) async fn get_non_outlier_pdu(&self, event_id: &EventId) -> Result<PduEvent> {
		let pduid = self.get_pdu_id(event_id).await?;

		self.pduid_pdu.get(&pduid).await.deserialized()
	}

	/// Like get_non_outlier_pdu(), but without the expense of fetching and
	/// parsing the PduEvent
	pub(super) async fn non_outlier_pdu_exists(&self, event_id: &EventId) -> Result {
		let pduid = self.get_pdu_id(event_id).await?;

		self.pduid_pdu.get(&pduid).await.map(|_| ())
	}

	/// Returns the pdu.
	///
	/// Checks the `eventid_outlierpdu` Tree if not found in the timeline.
	pub(super) async fn get_pdu(&self, event_id: &EventId) -> Result<Arc<PduEvent>> {
		if let Ok(pdu) = self.get_non_outlier_pdu(event_id).await {
			return Ok(Arc::new(pdu));
		}

		self.eventid_outlierpdu
			.get(event_id)
			.await
			.deserialized()
			.map(Arc::new)
	}

	/// Like get_non_outlier_pdu(), but without the expense of fetching and
	/// parsing the PduEvent
	pub(super) async fn outlier_pdu_exists(&self, event_id: &EventId) -> Result {
		self.eventid_outlierpdu.get(event_id).await.map(|_| ())
	}

	/// Like get_pdu(), but without the expense of fetching and parsing the data
	pub(super) async fn pdu_exists(&self, event_id: &EventId) -> bool {
		let non_outlier = self.non_outlier_pdu_exists(event_id).is_ok();
		let outlier = self.outlier_pdu_exists(event_id).is_ok();

		//TODO: parallelize
		non_outlier.await || outlier.await
	}

	/// Returns the pdu.
	///
	/// This does __NOT__ check the outliers `Tree`.
	pub(super) async fn get_pdu_from_id(&self, pdu_id: &[u8]) -> Result<PduEvent> {
		self.pduid_pdu.get(pdu_id).await.deserialized()
	}

	/// Returns the pdu as a `BTreeMap<String, CanonicalJsonValue>`.
	pub(super) async fn get_pdu_json_from_id(&self, pdu_id: &[u8]) -> Result<CanonicalJsonObject> {
		self.pduid_pdu.get(pdu_id).await.deserialized()
	}

	pub(super) async fn append_pdu(&self, pdu_id: &[u8], pdu: &PduEvent, json: &CanonicalJsonObject, count: u64) {
		self.pduid_pdu.raw_put(pdu_id, Json(json));
		self.lasttimelinecount_cache
			.lock()
			.await
			.insert(pdu.room_id.clone(), PduCount::Normal(count));

		self.eventid_pduid.insert(pdu.event_id.as_bytes(), pdu_id);
		self.eventid_outlierpdu.remove(pdu.event_id.as_bytes());
	}

	pub(super) fn prepend_backfill_pdu(&self, pdu_id: &[u8], event_id: &EventId, json: &CanonicalJsonObject) {
		self.pduid_pdu.raw_put(pdu_id, Json(json));
		self.eventid_pduid.insert(event_id, pdu_id);
		self.eventid_outlierpdu.remove(event_id);
	}

	/// Removes a pdu and creates a new one with the same id.
	pub(super) async fn replace_pdu(&self, pdu_id: &[u8], pdu_json: &CanonicalJsonObject, _pdu: &PduEvent) -> Result {
		if self.pduid_pdu.get(pdu_id).await.is_not_found() {
			return Err!(Request(NotFound("PDU does not exist.")));
		}

		self.pduid_pdu.raw_put(pdu_id, Json(pdu_json));

		Ok(())
	}

	/// Returns an iterator over all events and their tokens in a room that
	/// happened before the event with id `until` in reverse-chronological
	/// order.
	pub(super) async fn pdus_until<'a>(
		&'a self, user_id: &'a UserId, room_id: &'a RoomId, until: PduCount,
	) -> Result<impl Stream<Item = PdusIterItem> + Send + 'a> {
		let (prefix, current) = self.count_to_id(room_id, until, 1, true).await?;
		let stream = self
			.pduid_pdu
			.rev_raw_stream_from(&current)
			.ignore_err()
			.ready_take_while(move |(key, _)| key.starts_with(&prefix))
			.map(move |item| Self::each_pdu(item, user_id));

		Ok(stream)
	}

	pub(super) async fn pdus_after<'a>(
		&'a self, user_id: &'a UserId, room_id: &'a RoomId, from: PduCount,
	) -> Result<impl Stream<Item = PdusIterItem> + Send + 'a> {
		let (prefix, current) = self.count_to_id(room_id, from, 1, false).await?;
		let stream = self
			.pduid_pdu
			.raw_stream_from(&current)
			.ignore_err()
			.ready_take_while(move |(key, _)| key.starts_with(&prefix))
			.map(move |item| Self::each_pdu(item, user_id));

		Ok(stream)
	}

	fn each_pdu((pdu_id, pdu): KeyVal<'_>, user_id: &UserId) -> PdusIterItem {
		let mut pdu =
			serde_json::from_slice::<PduEvent>(pdu).expect("PduEvent in pduid_pdu database column is invalid JSON");

		if pdu.sender != user_id {
			pdu.remove_transaction_id().log_err().ok();
		}

		pdu.add_age().log_err().ok();
		let count = pdu_count(pdu_id);

		(count, pdu)
	}

	pub(super) fn increment_notification_counts(
		&self, room_id: &RoomId, notifies: Vec<OwnedUserId>, highlights: Vec<OwnedUserId>,
	) {
		let _cork = self.db.cork();

		for user in notifies {
			let mut userroom_id = user.as_bytes().to_vec();
			userroom_id.push(0xFF);
			userroom_id.extend_from_slice(room_id.as_bytes());
			increment(&self.userroomid_notificationcount, &userroom_id);
		}

		for user in highlights {
			let mut userroom_id = user.as_bytes().to_vec();
			userroom_id.push(0xFF);
			userroom_id.extend_from_slice(room_id.as_bytes());
			increment(&self.userroomid_highlightcount, &userroom_id);
		}
	}

	pub(super) async fn count_to_id(
		&self, room_id: &RoomId, count: PduCount, offset: u64, subtract: bool,
	) -> Result<(Vec<u8>, Vec<u8>)> {
		let prefix = self
			.services
			.short
			.get_shortroomid(room_id)
			.await
			.map_err(|e| err!(Request(NotFound("Room {room_id:?} not found: {e:?}"))))?
			.to_be_bytes()
			.to_vec();

		let mut pdu_id = prefix.clone();
		// +1 so we don't send the base event
		let count_raw = match count {
			PduCount::Normal(x) => {
				if subtract {
					x.saturating_sub(offset)
				} else {
					x.saturating_add(offset)
				}
			},
			PduCount::Backfilled(x) => {
				pdu_id.extend_from_slice(&0_u64.to_be_bytes());
				let num = u64::MAX.saturating_sub(x);
				if subtract {
					num.saturating_sub(offset)
				} else {
					num.saturating_add(offset)
				}
			},
		};
		pdu_id.extend_from_slice(&count_raw.to_be_bytes());

		Ok((prefix, pdu_id))
	}
}

/// Returns the `count` of this pdu's id.
pub(super) fn pdu_count(pdu_id: &[u8]) -> PduCount {
	const STRIDE: usize = size_of::<u64>();

	let pdu_id_len = pdu_id.len();
	let last_u64 = u64_from_u8(&pdu_id[expected!(pdu_id_len - STRIDE)..]);
	let second_last_u64 = u64_from_u8(&pdu_id[expected!(pdu_id_len - 2 * STRIDE)..expected!(pdu_id_len - STRIDE)]);

	if second_last_u64 == 0 {
		PduCount::Backfilled(u64::MAX.saturating_sub(last_u64))
	} else {
		PduCount::Normal(last_u64)
	}
}

//TODO: this is an ABA
fn increment(db: &Arc<Map>, key: &[u8]) {
	let old = db.get_blocking(key);
	let new = utils::increment(old.ok().as_deref());
	db.insert(key, new);
}
