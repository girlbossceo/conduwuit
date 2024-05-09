use std::{
	cmp,
	collections::{BTreeMap, HashMap, HashSet},
	fmt::Debug,
	sync::Arc,
	time::{Duration, Instant},
};

use base64::{engine::general_purpose, Engine as _};
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
	push, uint, MilliSecondsSinceUnixEpoch, OwnedServerName, OwnedUserId, RoomId, ServerName, UInt,
};
use tracing::{debug, error, warn};

use super::{appservice, send, Destination, Msg, SendingEvent, Service};
use crate::{service::presence::Presence, services, user_is_local, utils::calculate_hash, Error, PduEvent, Result};

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

impl Service {
	pub async fn start_handler(self: &Arc<Self>) {
		let self_ = Arc::clone(self);
		let handle = services().server.runtime().spawn(async move {
			self_
				.handler()
				.await
				.expect("Failed to start sending handler");
		});

		_ = self.handler_join.lock().await.insert(handle);
	}

	#[tracing::instrument(skip_all, name = "sender")]
	async fn handler(&self) -> Result<()> {
		let receiver = self.receiver.lock().await;
		let mut futures: SendingFutures<'_> = FuturesUnordered::new();
		let mut statuses: CurTransactionStatus = CurTransactionStatus::new();

		self.initial_transactions(&mut futures, &mut statuses);
		loop {
			debug_assert!(!receiver.is_closed(), "channel error");
			tokio::select! {
				request = receiver.recv_async() => match request {
					Ok(request) => self.handle_request(request, &mut futures, &mut statuses),
					Err(_) => return Ok(()),
				},
				Some(response) = futures.next() => {
					self.handle_response(response, &mut futures, &mut statuses);
				},
			}
		}
	}

	fn handle_response(
		&self, response: SendingResult, futures: &mut SendingFutures<'_>, statuses: &mut CurTransactionStatus,
	) {
		match response {
			Ok(dest) => self.handle_response_ok(&dest, futures, statuses),
			Err((dest, e)) => self.handle_response_err(dest, futures, statuses, &e),
		};
	}

	fn handle_response_err(
		&self, dest: Destination, _futures: &mut SendingFutures<'_>, statuses: &mut CurTransactionStatus, e: &Error,
	) {
		debug!(dest = ?dest, "{e:?}");
		statuses.entry(dest).and_modify(|e| {
			*e = match e {
				TransactionStatus::Running => TransactionStatus::Failed(1, Instant::now()),
				TransactionStatus::Retrying(n) => TransactionStatus::Failed(*n + 1, Instant::now()),
				TransactionStatus::Failed(..) => panic!("Request that was not even running failed?!"),
			}
		});
	}

	fn handle_response_ok(
		&self, dest: &Destination, futures: &mut SendingFutures<'_>, statuses: &mut CurTransactionStatus,
	) {
		let _cork = services().globals.db.cork();
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
			futures.push(Box::pin(send_events(dest.clone(), new_events_vec)));
		} else {
			statuses.remove(dest);
		}
	}

	fn handle_request(&self, msg: Msg, futures: &mut SendingFutures<'_>, statuses: &mut CurTransactionStatus) {
		let iv = vec![(msg.event, msg.queue_id)];
		if let Ok(Some(events)) = self.select_events(&msg.dest, iv, statuses) {
			if !events.is_empty() {
				futures.push(Box::pin(send_events(msg.dest, events)));
			} else {
				statuses.remove(&msg.dest);
			}
		}
	}

	fn initial_transactions(&self, futures: &mut SendingFutures<'_>, statuses: &mut CurTransactionStatus) {
		let keep = usize::try_from(self.startup_netburst_keep).unwrap_or(usize::MAX);
		let mut txns = HashMap::<Destination, Vec<SendingEvent>>::new();
		for (key, dest, event) in self.db.active_requests().filter_map(Result::ok) {
			let entry = txns.entry(dest.clone()).or_default();
			if self.startup_netburst_keep >= 0 && entry.len() >= keep {
				warn!("Dropping unsent event {:?} {:?}", dest, String::from_utf8_lossy(&key));
				self.db
					.delete_active_request(key)
					.expect("active request deleted");
			} else {
				entry.push(event);
			}
		}

		for (dest, events) in txns {
			if self.startup_netburst && !events.is_empty() {
				statuses.insert(dest.clone(), TransactionStatus::Running);
				futures.push(Box::pin(send_events(dest.clone(), events)));
			}
		}
	}

	#[tracing::instrument(skip(self, dest, new_events, statuses))]
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

		let _cork = services().globals.db.cork();
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
		let _cork = services().globals.db.cork();
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

	#[tracing::instrument(skip(self, dest, statuses))]
	fn select_events_current(&self, dest: Destination, statuses: &mut CurTransactionStatus) -> Result<(bool, bool)> {
		let (mut allow, mut retry) = (true, false);
		statuses
			.entry(dest)
			.and_modify(|e| match e {
				TransactionStatus::Failed(tries, time) => {
					// Fail if a request has failed recently (exponential backoff)
					let max_duration = Duration::from_secs(services().globals.config.sender_retry_backoff_limit);
					let min_duration = Duration::from_secs(services().globals.config.sender_timeout);
					let min_elapsed_duration = min_duration * (*tries) * (*tries);
					let min_elapsed_duration = cmp::min(min_elapsed_duration, max_duration);
					if time.elapsed() < min_elapsed_duration {
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

	#[tracing::instrument(skip(self, server_name))]
	fn select_edus(&self, server_name: &ServerName) -> Result<(Vec<Vec<u8>>, u64)> {
		// u64: count of last edu
		let since = self.db.get_latest_educount(server_name)?;
		let mut events = Vec::new();
		let mut max_edu_count = since;
		let mut device_list_changes = HashSet::new();

		for room_id in services().rooms.state_cache.server_rooms(server_name) {
			let room_id = room_id?;
			// Look for device list updates in this room
			device_list_changes.extend(
				services()
					.users
					.keys_changed(room_id.as_ref(), since, None)
					.filter_map(Result::ok)
					.filter(|user_id| user_is_local(user_id)),
			);

			if services().globals.allow_outgoing_read_receipts()
				&& !select_edus_receipts(&room_id, since, &mut max_edu_count, &mut events)?
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

		if services().globals.allow_outgoing_presence() {
			select_edus_presence(server_name, since, &mut max_edu_count, &mut events)?;
		}

		Ok((events, max_edu_count))
	}
}

/// Look for presence
#[tracing::instrument(skip(server_name, since, max_edu_count, events))]
fn select_edus_presence(
	server_name: &ServerName, since: u64, max_edu_count: &mut u64, events: &mut Vec<Vec<u8>>,
) -> Result<bool> {
	// Look for presence updates for this server
	let mut presence_updates = Vec::new();
	for (user_id, count, presence_bytes) in services().presence.presence_since(since) {
		*max_edu_count = cmp::max(count, *max_edu_count);

		if !user_is_local(&user_id) {
			continue;
		}

		if !services()
			.rooms
			.state_cache
			.server_sees_user(server_name, &user_id)?
		{
			continue;
		}

		let presence_event = Presence::from_json_bytes_to_event(&presence_bytes, &user_id)?;
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

	let presence_content = Edu::Presence(PresenceContent::new(presence_updates));
	events.push(serde_json::to_vec(&presence_content).expect("PresenceEvent can be serialized"));

	Ok(true)
}

/// Look for read receipts in this room
#[tracing::instrument(skip(room_id, since, max_edu_count, events))]
fn select_edus_receipts(
	room_id: &RoomId, since: u64, max_edu_count: &mut u64, events: &mut Vec<Vec<u8>>,
) -> Result<bool> {
	for r in services()
		.rooms
		.read_receipt
		.readreceipts_since(room_id, since)
	{
		let (user_id, count, read_receipt) = r?;
		*max_edu_count = cmp::max(count, *max_edu_count);

		if !user_is_local(&user_id) {
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

async fn send_events(dest: Destination, events: Vec<SendingEvent>) -> SendingResult {
	//debug_assert!(!events.is_empty(), "sending empty transaction");
	match dest {
		Destination::Normal(ref server) => send_events_dest_normal(&dest, server, events).await,
		Destination::Appservice(ref id) => send_events_dest_appservice(&dest, id, events).await,
		Destination::Push(ref userid, ref pushkey) => send_events_dest_push(&dest, userid, pushkey, events).await,
	}
}

#[tracing::instrument(skip(dest, events))]
async fn send_events_dest_appservice(dest: &Destination, id: &String, events: Vec<SendingEvent>) -> SendingResult {
	let mut pdu_jsons = Vec::new();

	for event in &events {
		match event {
			SendingEvent::Pdu(pdu_id) => {
				pdu_jsons.push(
					services()
						.rooms
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
	match appservice::send_request(
		services()
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

#[tracing::instrument(skip(dest, events))]
async fn send_events_dest_push(
	dest: &Destination, userid: &OwnedUserId, pushkey: &String, events: Vec<SendingEvent>,
) -> SendingResult {
	let mut pdus = Vec::new();

	for event in &events {
		match event {
			SendingEvent::Pdu(pdu_id) => {
				pdus.push(
					services()
						.rooms
						.timeline
						.get_pdu_from_id(pdu_id)
						.map_err(|e| (dest.clone(), e))?
						.ok_or_else(|| {
							(
								dest.clone(),
								Error::bad_database("[Push] Event in servernamevent_datas not found in db."),
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

		let Some(pusher) = services()
			.pusher
			.get_pusher(userid, pushkey)
			.map_err(|e| (dest.clone(), e))?
		else {
			continue;
		};

		let rules_for_user = services()
			.account_data
			.get(None, userid, GlobalAccountDataEventType::PushRules.to_string().into())
			.unwrap_or_default()
			.and_then(|event| serde_json::from_str::<PushRulesEvent>(event.get()).ok())
			.map_or_else(|| push::Ruleset::server_default(userid), |ev: PushRulesEvent| ev.content.global);

		let unread: UInt = services()
			.rooms
			.user
			.notification_count(userid, &pdu.room_id)
			.map_err(|e| (dest.clone(), e))?
			.try_into()
			.expect("notification count can't go that high");

		let _response = services()
			.pusher
			.send_push_notice(userid, unread, &pusher, rules_for_user, &pdu)
			.await
			.map(|_response| dest.clone())
			.map_err(|e| (dest.clone(), e));
	}

	Ok(dest.clone())
}

#[tracing::instrument(skip(dest, events), name = "")]
async fn send_events_dest_normal(
	dest: &Destination, server_name: &OwnedServerName, events: Vec<SendingEvent>,
) -> SendingResult {
	let mut edu_jsons = Vec::new();
	let mut pdu_jsons = Vec::new();

	for event in &events {
		match event {
			SendingEvent::Pdu(pdu_id) => {
				// TODO: check room version and remove event_id if needed
				let raw = PduEvent::convert_to_outgoing_federation_event(
					services()
						.rooms
						.timeline
						.get_pdu_json_from_id(pdu_id)
						.map_err(|e| (dest.clone(), e))?
						.ok_or_else(|| {
							error!(
								dest = ?dest,
								server_name = ?server_name,
								pdu_id = ?pdu_id,
								"event not found"
							);
							(
								dest.clone(),
								Error::bad_database("[Normal] Event in servernamevent_datas not found in db."),
							)
						})?,
				);
				pdu_jsons.push(raw);
			},
			SendingEvent::Edu(edu) => {
				if let Ok(raw) = serde_json::from_slice(edu) {
					edu_jsons.push(raw);
				}
			},
			SendingEvent::Flush => {
				// flush only; no new content
			},
		}
	}

	let client = &services().globals.client.sender;
	//debug_assert!(pdu_jsons.len() + edu_jsons.len() > 0, "sending empty
	// transaction");
	send::send(
		client,
		server_name,
		send_transaction_message::v1::Request {
			origin: services().globals.server_name().to_owned(),
			pdus: pdu_jsons,
			edus: edu_jsons,
			origin_server_ts: MilliSecondsSinceUnixEpoch::now(),
			transaction_id: (&*general_purpose::URL_SAFE_NO_PAD.encode(calculate_hash(
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
	.map(|response| {
		for pdu in response.pdus {
			if pdu.1.is_err() {
				warn!("error for {} from remote: {:?}", pdu.0, pdu.1);
			}
		}
		dest.clone()
	})
	.map_err(|e| (dest.clone(), e))
}
