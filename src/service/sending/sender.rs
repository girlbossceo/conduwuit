use std::{
	cmp,
	collections::{BTreeMap, HashMap, HashSet},
	fmt::Debug,
	time::{Duration, Instant},
};

use base64::{engine::general_purpose, Engine as _};
use conduit::{
	debug, debug_warn, error, trace,
	utils::{calculate_hash, math::continue_exponential_backoff_secs},
	warn, Error, Result,
};
use federation::transactions::send_transaction_message;
use futures_util::{future::BoxFuture, stream::FuturesUnordered, StreamExt};
use ruma::{
	api::federation::{
		self,
		transactions::edu::{
			DeviceListUpdateContent, Edu, PresenceContent, PresenceUpdate, ReceiptContent, ReceiptData, ReceiptMap,
		},
	},
	device_id,
	events::{push_rules::PushRulesEvent, receipt::ReceiptType, AnySyncEphemeralRoomEvent, GlobalAccountDataEventType},
	push, uint, CanonicalJsonObject, MilliSecondsSinceUnixEpoch, OwnedServerName, OwnedUserId, RoomId, RoomVersionId,
	ServerName, UInt,
};
use serde_json::value::{to_raw_value, RawValue as RawJsonValue};
use tokio::time::sleep_until;

use super::{appservice, Destination, Msg, SendingEvent, Service};

#[derive(Debug)]
enum TransactionStatus {
	Running,
	Failed(u32, Instant), // number of times failed, time of last failure
	Retrying(u32),        // number of times failed
}

type SendingError = (Destination, Error);
type SendingResult = Result<Destination, SendingError>;
type SendingFuture<'a> = BoxFuture<'a, SendingResult>;
type SendingFutures<'a> = FuturesUnordered<SendingFuture<'a>>;
type CurTransactionStatus = HashMap<Destination, TransactionStatus>;

const DEQUEUE_LIMIT: usize = 48;
const SELECT_EDU_LIMIT: usize = 16;
const CLEANUP_TIMEOUT_MS: u64 = 3500;

impl Service {
	#[tracing::instrument(skip_all, name = "sender")]
	pub(super) async fn sender(&self) -> Result<()> {
		let receiver = self.receiver.lock().await;
		let mut futures: SendingFutures<'_> = FuturesUnordered::new();
		let mut statuses: CurTransactionStatus = CurTransactionStatus::new();

		self.initial_requests(&futures, &mut statuses);
		loop {
			debug_assert!(!receiver.is_closed(), "channel error");
			tokio::select! {
				request = receiver.recv_async() => match request {
					Ok(request) => self.handle_request(request, &futures, &mut statuses),
					Err(_) => break,
				},
				Some(response) = futures.next() => {
					self.handle_response(response, &futures, &mut statuses);
				},
			}
		}
		self.finish_responses(&mut futures, &mut statuses).await;

		Ok(())
	}

	fn handle_response<'a>(
		&'a self, response: SendingResult, futures: &SendingFutures<'a>, statuses: &mut CurTransactionStatus,
	) {
		match response {
			Ok(dest) => self.handle_response_ok(&dest, futures, statuses),
			Err((dest, e)) => Self::handle_response_err(dest, futures, statuses, &e),
		};
	}

	fn handle_response_err(
		dest: Destination, _futures: &SendingFutures<'_>, statuses: &mut CurTransactionStatus, e: &Error,
	) {
		debug!(dest = ?dest, "{e:?}");
		statuses.entry(dest).and_modify(|e| {
			*e = match e {
				TransactionStatus::Running => TransactionStatus::Failed(1, Instant::now()),
				TransactionStatus::Retrying(ref n) => TransactionStatus::Failed(n.saturating_add(1), Instant::now()),
				TransactionStatus::Failed(..) => panic!("Request that was not even running failed?!"),
			}
		});
	}

	fn handle_response_ok<'a>(
		&'a self, dest: &Destination, futures: &SendingFutures<'a>, statuses: &mut CurTransactionStatus,
	) {
		let _cork = self.db.db.cork();
		self.db
			.delete_all_active_requests_for(dest)
			.expect("all active requests deleted");

		// Find events that have been added since starting the last request
		let new_events = self
			.db
			.queued_requests(dest)
			.filter_map(Result::ok)
			.take(DEQUEUE_LIMIT)
			.collect::<Vec<_>>();

		// Insert any pdus we found
		if !new_events.is_empty() {
			self.db
				.mark_as_active(&new_events)
				.expect("marked as active");
			let new_events_vec = new_events.into_iter().map(|(event, _)| event).collect();
			futures.push(Box::pin(self.send_events(dest.clone(), new_events_vec)));
		} else {
			statuses.remove(dest);
		}
	}

	fn handle_request<'a>(&'a self, msg: Msg, futures: &SendingFutures<'a>, statuses: &mut CurTransactionStatus) {
		let iv = vec![(msg.event, msg.queue_id)];
		if let Ok(Some(events)) = self.select_events(&msg.dest, iv, statuses) {
			if !events.is_empty() {
				futures.push(Box::pin(self.send_events(msg.dest, events)));
			} else {
				statuses.remove(&msg.dest);
			}
		}
	}

	async fn finish_responses<'a>(&'a self, futures: &mut SendingFutures<'a>, statuses: &mut CurTransactionStatus) {
		let now = Instant::now();
		let timeout = Duration::from_millis(CLEANUP_TIMEOUT_MS);
		let deadline = now.checked_add(timeout).unwrap_or(now);
		loop {
			trace!("Waiting for {} requests to complete...", futures.len());
			tokio::select! {
				() = sleep_until(deadline.into()) => break,
				response = futures.next() => match response {
					Some(response) => self.handle_response(response, futures, statuses),
					None => return,
				}
			}
		}

		debug_warn!("Leaving with {} unfinished requests...", futures.len());
	}

	fn initial_requests<'a>(&'a self, futures: &SendingFutures<'a>, statuses: &mut CurTransactionStatus) {
		let keep = usize::try_from(self.server.config.startup_netburst_keep).unwrap_or(usize::MAX);
		let mut txns = HashMap::<Destination, Vec<SendingEvent>>::new();
		for (key, dest, event) in self.db.active_requests().filter_map(Result::ok) {
			let entry = txns.entry(dest.clone()).or_default();
			if self.server.config.startup_netburst_keep >= 0 && entry.len() >= keep {
				warn!("Dropping unsent event {:?} {:?}", dest, String::from_utf8_lossy(&key));
				self.db
					.delete_active_request(&key)
					.expect("active request deleted");
			} else {
				entry.push(event);
			}
		}

		for (dest, events) in txns {
			if self.server.config.startup_netburst && !events.is_empty() {
				statuses.insert(dest.clone(), TransactionStatus::Running);
				futures.push(Box::pin(self.send_events(dest.clone(), events)));
			}
		}
	}

	#[tracing::instrument(skip_all, level = "debug")]
	fn select_events(
		&self,
		dest: &Destination,
		new_events: Vec<(SendingEvent, Vec<u8>)>, // Events we want to send: event and full key
		statuses: &mut CurTransactionStatus,
	) -> Result<Option<Vec<SendingEvent>>> {
		let (allow, retry) = self.select_events_current(dest.clone(), statuses)?;

		// Nothing can be done for this remote, bail out.
		if !allow {
			return Ok(None);
		}

		let _cork = self.db.db.cork();
		let mut events = Vec::new();

		// Must retry any previous transaction for this remote.
		if retry {
			self.db
				.active_requests_for(dest)
				.filter_map(Result::ok)
				.for_each(|(_, e)| events.push(e));

			return Ok(Some(events));
		}

		// Compose the next transaction
		let _cork = self.db.db.cork();
		if !new_events.is_empty() {
			self.db.mark_as_active(&new_events)?;
			for (e, _) in new_events {
				events.push(e);
			}
		}

		// Add EDU's into the transaction
		if let Destination::Normal(server_name) = dest {
			if let Ok((select_edus, last_count)) = self.select_edus(server_name) {
				events.extend(select_edus.into_iter().map(SendingEvent::Edu));
				self.db.set_latest_educount(server_name, last_count)?;
			}
		}

		Ok(Some(events))
	}

	#[tracing::instrument(skip_all, level = "debug")]
	fn select_events_current(&self, dest: Destination, statuses: &mut CurTransactionStatus) -> Result<(bool, bool)> {
		let (mut allow, mut retry) = (true, false);
		statuses
			.entry(dest)
			.and_modify(|e| match e {
				TransactionStatus::Failed(tries, time) => {
					// Fail if a request has failed recently (exponential backoff)
					let min = self.server.config.sender_timeout;
					let max = self.server.config.sender_retry_backoff_limit;
					if continue_exponential_backoff_secs(min, max, time.elapsed(), *tries) {
						allow = false;
					} else {
						retry = true;
						*e = TransactionStatus::Retrying(*tries);
					}
				},
				TransactionStatus::Running | TransactionStatus::Retrying(_) => {
					allow = false; // already running
				},
			})
			.or_insert(TransactionStatus::Running);

		Ok((allow, retry))
	}

	#[tracing::instrument(skip_all, level = "debug")]
	fn select_edus(&self, server_name: &ServerName) -> Result<(Vec<Vec<u8>>, u64)> {
		// u64: count of last edu
		let since = self.db.get_latest_educount(server_name)?;
		let mut events = Vec::new();
		let mut max_edu_count = since;
		let mut device_list_changes = HashSet::new();

		for room_id in self.services.state_cache.server_rooms(server_name) {
			let room_id = room_id?;
			// Look for device list updates in this room
			device_list_changes.extend(
				self.services
					.users
					.keys_changed(room_id.as_ref(), since, None)
					.filter_map(Result::ok)
					.filter(|user_id| self.services.globals.user_is_local(user_id)),
			);

			if self.server.config.allow_outgoing_read_receipts
				&& !self.select_edus_receipts(&room_id, since, &mut max_edu_count, &mut events)?
			{
				break;
			}
		}

		for user_id in device_list_changes {
			// Empty prev id forces synapse to resync; because synapse resyncs,
			// we can just insert placeholder data
			let edu = Edu::DeviceListUpdate(DeviceListUpdateContent {
				user_id,
				device_id: device_id!("placeholder").to_owned(),
				device_display_name: Some("Placeholder".to_owned()),
				stream_id: uint!(1),
				prev_id: Vec::new(),
				deleted: None,
				keys: None,
			});

			events.push(serde_json::to_vec(&edu).expect("json can be serialized"));
		}

		if self.server.config.allow_outgoing_presence {
			self.select_edus_presence(server_name, since, &mut max_edu_count, &mut events)?;
		}

		Ok((events, max_edu_count))
	}

	/// Look for presence
	fn select_edus_presence(
		&self, server_name: &ServerName, since: u64, max_edu_count: &mut u64, events: &mut Vec<Vec<u8>>,
	) -> Result<bool> {
		// Look for presence updates for this server
		let mut presence_updates = Vec::new();
		for (user_id, count, presence_bytes) in self.services.presence.presence_since(since) {
			*max_edu_count = cmp::max(count, *max_edu_count);

			if !self.services.globals.user_is_local(&user_id) {
				continue;
			}

			if !self
				.services
				.state_cache
				.server_sees_user(server_name, &user_id)?
			{
				continue;
			}

			let presence_event = self
				.services
				.presence
				.from_json_bytes_to_event(&presence_bytes, &user_id)?;
			presence_updates.push(PresenceUpdate {
				user_id,
				presence: presence_event.content.presence,
				currently_active: presence_event.content.currently_active.unwrap_or(false),
				last_active_ago: presence_event
					.content
					.last_active_ago
					.unwrap_or_else(|| uint!(0)),
				status_msg: presence_event.content.status_msg,
			});

			if presence_updates.len() >= SELECT_EDU_LIMIT {
				break;
			}
		}

		if !presence_updates.is_empty() {
			let presence_content = Edu::Presence(PresenceContent::new(presence_updates));
			events.push(serde_json::to_vec(&presence_content).expect("PresenceEvent can be serialized"));
		}

		Ok(true)
	}

	/// Look for read receipts in this room
	fn select_edus_receipts(
		&self, room_id: &RoomId, since: u64, max_edu_count: &mut u64, events: &mut Vec<Vec<u8>>,
	) -> Result<bool> {
		for r in self
			.services
			.read_receipt
			.readreceipts_since(room_id, since)
		{
			let (user_id, count, read_receipt) = r?;
			*max_edu_count = cmp::max(count, *max_edu_count);

			if !self.services.globals.user_is_local(&user_id) {
				continue;
			}

			let event = serde_json::from_str(read_receipt.json().get())
				.map_err(|_| Error::bad_database("Invalid edu event in read_receipts."))?;
			let federation_event = if let AnySyncEphemeralRoomEvent::Receipt(r) = event {
				let mut read = BTreeMap::new();

				let (event_id, mut receipt) = r
					.content
					.0
					.into_iter()
					.next()
					.expect("we only use one event per read receipt");
				let receipt = receipt
					.remove(&ReceiptType::Read)
					.expect("our read receipts always set this")
					.remove(&user_id)
					.expect("our read receipts always have the user here");

				read.insert(
					user_id,
					ReceiptData {
						data: receipt.clone(),
						event_ids: vec![event_id.clone()],
					},
				);

				let receipt_map = ReceiptMap {
					read,
				};

				let mut receipts = BTreeMap::new();
				receipts.insert(room_id.to_owned(), receipt_map);

				Edu::Receipt(ReceiptContent {
					receipts,
				})
			} else {
				Error::bad_database("Invalid event type in read_receipts");
				continue;
			};

			events.push(serde_json::to_vec(&federation_event).expect("json can be serialized"));

			if events.len() >= SELECT_EDU_LIMIT {
				return Ok(false);
			}
		}

		Ok(true)
	}

	async fn send_events(&self, dest: Destination, events: Vec<SendingEvent>) -> SendingResult {
		//debug_assert!(!events.is_empty(), "sending empty transaction");
		match dest {
			Destination::Normal(ref server) => self.send_events_dest_normal(&dest, server, events).await,
			Destination::Appservice(ref id) => self.send_events_dest_appservice(&dest, id, events).await,
			Destination::Push(ref userid, ref pushkey) => {
				self.send_events_dest_push(&dest, userid, pushkey, events)
					.await
			},
		}
	}

	#[tracing::instrument(skip(self, dest, events), name = "appservice")]
	async fn send_events_dest_appservice(
		&self, dest: &Destination, id: &str, events: Vec<SendingEvent>,
	) -> SendingResult {
		let mut pdu_jsons = Vec::new();

		for event in &events {
			match event {
				SendingEvent::Pdu(pdu_id) => {
					pdu_jsons.push(
						self.services
							.timeline
							.get_pdu_from_id(pdu_id)
							.map_err(|e| (dest.clone(), e))?
							.ok_or_else(|| {
								(
									dest.clone(),
									Error::bad_database("[Appservice] Event in servernameevent_data not found in db."),
								)
							})?
							.to_room_event(),
					);
				},
				SendingEvent::Edu(_) | SendingEvent::Flush => {
					// Appservices don't need EDUs (?) and flush only;
					// no new content
				},
			}
		}

		//debug_assert!(!pdu_jsons.is_empty(), "sending empty transaction");
		let client = &self.services.client.appservice;
		match appservice::send_request(
			client,
			self.services
				.appservice
				.get_registration(id)
				.await
				.ok_or_else(|| {
					(
						dest.clone(),
						Error::bad_database("[Appservice] Could not load registration from db."),
					)
				})?,
			ruma::api::appservice::event::push_events::v1::Request {
				events: pdu_jsons,
				txn_id: (&*general_purpose::URL_SAFE_NO_PAD.encode(calculate_hash(
					&events
						.iter()
						.map(|e| match e {
							SendingEvent::Edu(b) | SendingEvent::Pdu(b) => &**b,
							SendingEvent::Flush => &[],
						})
						.collect::<Vec<_>>(),
				)))
					.into(),
			},
		)
		.await
		{
			Ok(_) => Ok(dest.clone()),
			Err(e) => Err((dest.clone(), e)),
		}
	}

	#[tracing::instrument(skip(self, dest, events), name = "push")]
	async fn send_events_dest_push(
		&self, dest: &Destination, userid: &OwnedUserId, pushkey: &str, events: Vec<SendingEvent>,
	) -> SendingResult {
		let mut pdus = Vec::new();

		for event in &events {
			match event {
				SendingEvent::Pdu(pdu_id) => {
					pdus.push(
						self.services
							.timeline
							.get_pdu_from_id(pdu_id)
							.map_err(|e| (dest.clone(), e))?
							.ok_or_else(|| {
								(
									dest.clone(),
									Error::bad_database("[Push] Event in servernameevent_data not found in db."),
								)
							})?,
					);
				},
				SendingEvent::Edu(_) | SendingEvent::Flush => {
					// Push gateways don't need EDUs (?) and flush only;
					// no new content
				},
			}
		}

		for pdu in pdus {
			// Redacted events are not notification targets (we don't send push for them)
			if let Some(unsigned) = &pdu.unsigned {
				if let Ok(unsigned) = serde_json::from_str::<serde_json::Value>(unsigned.get()) {
					if unsigned.get("redacted_because").is_some() {
						continue;
					}
				}
			}

			let Some(pusher) = self
				.services
				.pusher
				.get_pusher(userid, pushkey)
				.map_err(|e| (dest.clone(), e))?
			else {
				continue;
			};

			let rules_for_user = self
				.services
				.account_data
				.get(None, userid, GlobalAccountDataEventType::PushRules.to_string().into())
				.unwrap_or_default()
				.and_then(|event| serde_json::from_str::<PushRulesEvent>(event.get()).ok())
				.map_or_else(|| push::Ruleset::server_default(userid), |ev: PushRulesEvent| ev.content.global);

			let unread: UInt = self
				.services
				.user
				.notification_count(userid, &pdu.room_id)
				.map_err(|e| (dest.clone(), e))?
				.try_into()
				.expect("notification count can't go that high");

			let _response = self
				.services
				.pusher
				.send_push_notice(userid, unread, &pusher, rules_for_user, &pdu)
				.await
				.map(|_response| dest.clone())
				.map_err(|e| (dest.clone(), e));
		}

		Ok(dest.clone())
	}

	#[tracing::instrument(skip(self, dest, events), name = "", level = "debug")]
	async fn send_events_dest_normal(
		&self, dest: &Destination, server: &OwnedServerName, events: Vec<SendingEvent>,
	) -> SendingResult {
		let mut pdu_jsons = Vec::with_capacity(
			events
				.iter()
				.filter(|event| matches!(event, SendingEvent::Pdu(_)))
				.count(),
		);
		let mut edu_jsons = Vec::with_capacity(
			events
				.iter()
				.filter(|event| matches!(event, SendingEvent::Edu(_)))
				.count(),
		);

		for event in &events {
			match event {
				// TODO: check room version and remove event_id if needed
				SendingEvent::Pdu(pdu_id) => pdu_jsons.push(
					self.convert_to_outgoing_federation_event(
						self.services
							.timeline
							.get_pdu_json_from_id(pdu_id)
							.map_err(|e| (dest.clone(), e))?
							.ok_or_else(|| {
								error!(?dest, ?server, ?pdu_id, "event not found");
								(
									dest.clone(),
									Error::bad_database("[Normal] Event in servernameevent_data not found in db."),
								)
							})?,
					),
				),
				SendingEvent::Edu(edu) => {
					if let Ok(raw) = serde_json::from_slice(edu) {
						edu_jsons.push(raw);
					}
				},
				SendingEvent::Flush => {}, // flush only; no new content
			}
		}

		//debug_assert!(pdu_jsons.len() + edu_jsons.len() > 0, "sending empty
		// transaction");
		let transaction_id = &*general_purpose::URL_SAFE_NO_PAD.encode(calculate_hash(
			&events
				.iter()
				.map(|e| match e {
					SendingEvent::Edu(b) | SendingEvent::Pdu(b) => &**b,
					SendingEvent::Flush => &[],
				})
				.collect::<Vec<_>>(),
		));

		let request = send_transaction_message::v1::Request {
			origin: self.server.config.server_name.clone(),
			pdus: pdu_jsons,
			edus: edu_jsons,
			origin_server_ts: MilliSecondsSinceUnixEpoch::now(),
			transaction_id: transaction_id.into(),
		};

		let client = &self.services.client.sender;
		self.send(client, server, request)
			.await
			.inspect(|response| {
				response
					.pdus
					.iter()
					.filter(|(_, res)| res.is_err())
					.for_each(|(pdu_id, res)| warn!("error for {pdu_id} from remote: {res:?}"));
			})
			.map(|_| dest.clone())
			.map_err(|e| (dest.clone(), e))
	}

	/// This does not return a full `Pdu` it is only to satisfy ruma's types.
	pub fn convert_to_outgoing_federation_event(&self, mut pdu_json: CanonicalJsonObject) -> Box<RawJsonValue> {
		if let Some(unsigned) = pdu_json
			.get_mut("unsigned")
			.and_then(|val| val.as_object_mut())
		{
			unsigned.remove("transaction_id");
		}

		// room v3 and above removed the "event_id" field from remote PDU format
		if let Some(room_id) = pdu_json
			.get("room_id")
			.and_then(|val| RoomId::parse(val.as_str()?).ok())
		{
			match self.services.state.get_room_version(&room_id) {
				Ok(room_version_id) => match room_version_id {
					RoomVersionId::V1 | RoomVersionId::V2 => {},
					_ => _ = pdu_json.remove("event_id"),
				},
				Err(_) => _ = pdu_json.remove("event_id"),
			}
		} else {
			pdu_json.remove("event_id");
		}

		// TODO: another option would be to convert it to a canonical string to validate
		// size and return a Result<Raw<...>>
		// serde_json::from_str::<Raw<_>>(
		//     ruma::serde::to_canonical_json_string(pdu_json).expect("CanonicalJson is
		// valid serde_json::Value"), )
		// .expect("Raw::from_value always works")

		to_raw_value(&pdu_json).expect("CanonicalJson is valid serde_json::Value")
	}
}
