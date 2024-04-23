use std::{
	cmp,
	collections::{BTreeMap, HashMap, HashSet},
	fmt::Debug,
	sync::Arc,
	time::{Duration, Instant},
};

use base64::{engine::general_purpose, Engine as _};
pub(crate) use data::Data;
use federation::transactions::send_transaction_message;
use futures_util::{stream::FuturesUnordered, StreamExt};
use ruma::{
	api::{
		appservice::Registration,
		federation::{
			self,
			transactions::edu::{
				DeviceListUpdateContent, Edu, PresenceContent, PresenceUpdate, ReceiptContent, ReceiptData, ReceiptMap,
			},
		},
		OutgoingRequest,
	},
	device_id,
	events::{push_rules::PushRulesEvent, receipt::ReceiptType, AnySyncEphemeralRoomEvent, GlobalAccountDataEventType},
	push, uint, MilliSecondsSinceUnixEpoch, OwnedServerName, OwnedUserId, RoomId, ServerName, UInt, UserId,
};
use tokio::sync::{Mutex, Semaphore};
use tracing::{error, warn};

use crate::{service::presence::Presence, services, utils::calculate_hash, Config, Error, PduEvent, Result};

mod appservice;
mod data;
mod send;
pub(crate) use send::FedDest;

const SELECT_EDU_LIMIT: usize = 16;

pub(crate) struct Service {
	pub(crate) db: &'static dyn Data,

	/// The state for a given state hash.
	pub(crate) maximum_requests: Arc<Semaphore>,
	pub(crate) sender: loole::Sender<(Destination, SendingEventType, Vec<u8>)>,
	receiver: Mutex<loole::Receiver<(Destination, SendingEventType, Vec<u8>)>>,
	startup_netburst: bool,
	startup_netburst_keep: i64,
	timeout: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Destination {
	Appservice(String),
	Push(OwnedUserId, String), // user and pushkey
	Normal(OwnedServerName),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[allow(clippy::module_name_repetitions)]
pub(crate) enum SendingEventType {
	Pdu(Vec<u8>), // pduid
	Edu(Vec<u8>), // pdu json
	Flush,        // none
}

enum TransactionStatus {
	Running,
	Failed(u32, Instant), // number of times failed, time of last failure
	Retrying(u32),        // number of times failed
}

impl Service {
	pub(crate) fn build(db: &'static dyn Data, config: &Config) -> Arc<Self> {
		let (sender, receiver) = loole::unbounded();
		Arc::new(Self {
			db,
			sender,
			receiver: Mutex::new(receiver),
			maximum_requests: Arc::new(Semaphore::new(config.max_concurrent_requests as usize)),
			startup_netburst: config.startup_netburst,
			startup_netburst_keep: config.startup_netburst_keep,
			timeout: config.sender_timeout,
		})
	}

	#[tracing::instrument(skip(self, pdu_id, user, pushkey))]
	pub(crate) fn send_pdu_push(&self, pdu_id: &[u8], user: &UserId, pushkey: String) -> Result<()> {
		let dest = Destination::Push(user.to_owned(), pushkey);
		let event = SendingEventType::Pdu(pdu_id.to_owned());
		let _cork = services().globals.db.cork()?;
		let keys = self.db.queue_requests(&[(&dest, event.clone())])?;
		self.sender
			.send((dest, event, keys.into_iter().next().unwrap()))
			.unwrap();

		Ok(())
	}

	#[tracing::instrument(skip(self))]
	pub(crate) fn send_pdu_appservice(&self, appservice_id: String, pdu_id: Vec<u8>) -> Result<()> {
		let dest = Destination::Appservice(appservice_id);
		let event = SendingEventType::Pdu(pdu_id);
		let _cork = services().globals.db.cork()?;
		let keys = self.db.queue_requests(&[(&dest, event.clone())])?;
		self.sender
			.send((dest, event, keys.into_iter().next().unwrap()))
			.unwrap();

		Ok(())
	}

	#[tracing::instrument(skip(self, room_id, pdu_id))]
	pub(crate) fn send_pdu_room(&self, room_id: &RoomId, pdu_id: &[u8]) -> Result<()> {
		let servers = services()
			.rooms
			.state_cache
			.room_servers(room_id)
			.filter_map(Result::ok)
			.filter(|server| &**server != services().globals.server_name());

		self.send_pdu_servers(servers, pdu_id)
	}

	#[tracing::instrument(skip(self, servers, pdu_id))]
	pub(crate) fn send_pdu_servers<I: Iterator<Item = OwnedServerName>>(
		&self, servers: I, pdu_id: &[u8],
	) -> Result<()> {
		let requests = servers
			.into_iter()
			.map(|server| (Destination::Normal(server), SendingEventType::Pdu(pdu_id.to_owned())))
			.collect::<Vec<_>>();
		let _cork = services().globals.db.cork()?;
		let keys = self.db.queue_requests(
			&requests
				.iter()
				.map(|(o, e)| (o, e.clone()))
				.collect::<Vec<_>>(),
		)?;
		for ((dest, event), key) in requests.into_iter().zip(keys) {
			self.sender.send((dest.clone(), event, key)).unwrap();
		}

		Ok(())
	}

	#[tracing::instrument(skip(self, server, serialized))]
	pub(crate) fn send_edu_server(&self, server: &ServerName, serialized: Vec<u8>) -> Result<()> {
		let dest = Destination::Normal(server.to_owned());
		let event = SendingEventType::Edu(serialized);
		let _cork = services().globals.db.cork()?;
		let keys = self.db.queue_requests(&[(&dest, event.clone())])?;
		self.sender
			.send((dest, event, keys.into_iter().next().unwrap()))
			.unwrap();

		Ok(())
	}

	#[tracing::instrument(skip(self, room_id, serialized))]
	pub(crate) fn send_edu_room(&self, room_id: &RoomId, serialized: Vec<u8>) -> Result<()> {
		let servers = services()
			.rooms
			.state_cache
			.room_servers(room_id)
			.filter_map(Result::ok)
			.filter(|server| &**server != services().globals.server_name());

		self.send_edu_servers(servers, serialized)
	}

	#[tracing::instrument(skip(self, servers, serialized))]
	pub(crate) fn send_edu_servers<I: Iterator<Item = OwnedServerName>>(
		&self, servers: I, serialized: Vec<u8>,
	) -> Result<()> {
		let requests = servers
			.into_iter()
			.map(|server| (Destination::Normal(server), SendingEventType::Edu(serialized.clone())))
			.collect::<Vec<_>>();
		let _cork = services().globals.db.cork()?;
		let keys = self.db.queue_requests(
			&requests
				.iter()
				.map(|(o, e)| (o, e.clone()))
				.collect::<Vec<_>>(),
		)?;
		for ((dest, event), key) in requests.into_iter().zip(keys) {
			self.sender.send((dest.clone(), event, key)).unwrap();
		}

		Ok(())
	}

	#[tracing::instrument(skip(self, room_id))]
	pub(crate) fn flush_room(&self, room_id: &RoomId) -> Result<()> {
		let servers = services()
			.rooms
			.state_cache
			.room_servers(room_id)
			.filter_map(Result::ok)
			.filter(|server| &**server != services().globals.server_name());

		self.flush_servers(servers)
	}

	#[tracing::instrument(skip(self, servers))]
	pub(crate) fn flush_servers<I: Iterator<Item = OwnedServerName>>(&self, servers: I) -> Result<()> {
		let requests = servers.into_iter().map(Destination::Normal);

		for dest in requests {
			self.sender
				.send((dest, SendingEventType::Flush, Vec::<u8>::new()))
				.unwrap();
		}

		Ok(())
	}

	/// Cleanup event data
	/// Used for instance after we remove an appservice registration
	#[tracing::instrument(skip(self))]
	pub(crate) fn cleanup_events(&self, appservice_id: String) -> Result<()> {
		self.db
			.delete_all_requests_for(&Destination::Appservice(appservice_id))?;

		Ok(())
	}

	#[tracing::instrument(skip(self, request), name = "request")]
	pub(crate) async fn send_federation_request<T>(&self, dest: &ServerName, request: T) -> Result<T::IncomingResponse>
	where
		T: OutgoingRequest + Debug,
	{
		let permit = self.maximum_requests.acquire().await;
		let timeout = Duration::from_secs(self.timeout);
		let client = &services().globals.client.federation;
		let response = tokio::time::timeout(timeout, send::send_request(client, dest, request))
			.await
			.map_err(|_| {
				warn!("Timeout after 300 seconds waiting for server response of {dest}");
				Error::BadServerResponse("Timeout after 300 seconds waiting for server response")
			})?;
		drop(permit);

		response
	}

	/// Sends a request to an appservice
	///
	/// Only returns None if there is no url specified in the appservice
	/// registration file
	pub(crate) async fn send_appservice_request<T>(
		&self, registration: Registration, request: T,
	) -> Result<Option<T::IncomingResponse>>
	where
		T: OutgoingRequest + Debug,
	{
		let permit = self.maximum_requests.acquire().await;
		let response = appservice::send_request(registration, request).await;
		drop(permit);

		response
	}

	pub(crate) fn start_handler(self: &Arc<Self>) {
		let self2 = Arc::clone(self);
		tokio::spawn(async move {
			self2
				.handler()
				.await
				.expect("Failed to initialize request sending handler");
		});
	}

	#[tracing::instrument(skip_all, name = "sender")]
	async fn handler(&self) -> Result<()> {
		let receiver = self.receiver.lock().await;

		let mut futures = FuturesUnordered::new();
		let mut current_transaction_status = HashMap::<Destination, TransactionStatus>::new();

		// Retry requests we could not finish yet
		if self.startup_netburst {
			let mut initial_transactions = HashMap::<Destination, Vec<SendingEventType>>::new();
			for (key, dest, event) in self.db.active_requests().filter_map(Result::ok) {
				let entry = initial_transactions.entry(dest.clone()).or_default();

				if self.startup_netburst_keep >= 0
					&& entry.len() >= usize::try_from(self.startup_netburst_keep).unwrap()
				{
					warn!("Dropping unsent event {:?} {:?}", dest, String::from_utf8_lossy(&key),);
					self.db.delete_active_request(key)?;
					continue;
				}

				entry.push(event);
			}

			for (dest, events) in initial_transactions {
				current_transaction_status.insert(dest.clone(), TransactionStatus::Running);
				futures.push(send_events(dest.clone(), events));
			}
		}

		loop {
			tokio::select! {
				Some(response) = futures.next() => {
					match response {
						Ok(dest) => {
							let _cork = services().globals.db.cork();
							self.db.delete_all_active_requests_for(&dest)?;

							// Find events that have been added since starting the last request
							let new_events = self
								.db
								.queued_requests(&dest)
								.filter_map(Result::ok)
								.take(30).collect::<Vec<_>>();

							if !new_events.is_empty() {
								// Insert pdus we found
								self.db.mark_as_active(&new_events)?;
								futures.push(send_events(
									dest.clone(),
									new_events.into_iter().map(|(event, _)| event).collect(),
								));
							} else {
								current_transaction_status.remove(&dest);
							}
						}
						Err((dest, _)) => {
							current_transaction_status.entry(dest).and_modify(|e| *e = match e {
								TransactionStatus::Running => TransactionStatus::Failed(1, Instant::now()),
								TransactionStatus::Retrying(n) => TransactionStatus::Failed(*n+1, Instant::now()),
								TransactionStatus::Failed(_, _) => {
									error!("Request that was not even running failed?!");
									return
								},
							});
						}
					};
				},

				event = receiver.recv_async() => {
					if let Ok((dest, event, key)) = event {
						if let Ok(Some(events)) = self.select_events(
							&dest,
							vec![(event, key)],
							&mut current_transaction_status,
						) {
							futures.push(send_events(dest, events));
						}
					}
				}
			}
		}
	}

	#[tracing::instrument(skip(self, dest, new_events, current_transaction_status))]
	fn select_events(
		&self,
		dest: &Destination,
		new_events: Vec<(SendingEventType, Vec<u8>)>, // Events we want to send: event and full key
		current_transaction_status: &mut HashMap<Destination, TransactionStatus>,
	) -> Result<Option<Vec<SendingEventType>>> {
		let (allow, retry) = self.select_events_current(dest.clone(), current_transaction_status)?;

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
				events.extend(select_edus.into_iter().map(SendingEventType::Edu));
				self.db.set_latest_educount(server_name, last_count)?;
			}
		}

		Ok(Some(events))
	}

	#[tracing::instrument(skip(self, dest, current_transaction_status))]
	fn select_events_current(
		&self, dest: Destination, current_transaction_status: &mut HashMap<Destination, TransactionStatus>,
	) -> Result<(bool, bool)> {
		let (mut allow, mut retry) = (true, false);
		current_transaction_status
			.entry(dest)
			.and_modify(|e| match e {
				TransactionStatus::Failed(tries, time) => {
					// Fail if a request has failed recently (exponential backoff)
					let min_duration = Duration::from_secs(services().globals.config.sender_timeout);
					let max_duration = Duration::from_secs(services().globals.config.sender_retry_backoff_limit);
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
	pub(crate) fn select_edus(&self, server_name: &ServerName) -> Result<(Vec<Vec<u8>>, u64)> {
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
					.filter(|user_id| user_id.server_name() == services().globals.server_name()),
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
pub(crate) fn select_edus_presence(
	server_name: &ServerName, since: u64, max_edu_count: &mut u64, events: &mut Vec<Vec<u8>>,
) -> Result<bool> {
	// Look for presence updates for this server
	let mut presence_updates = Vec::new();
	for (user_id, count, presence_bytes) in services().presence.presence_since(since) {
		*max_edu_count = cmp::max(count, *max_edu_count);

		if user_id.server_name() != services().globals.server_name() {
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
pub(crate) fn select_edus_receipts(
	room_id: &RoomId, since: u64, max_edu_count: &mut u64, events: &mut Vec<Vec<u8>>,
) -> Result<bool> {
	for r in services()
		.rooms
		.read_receipt
		.readreceipts_since(room_id, since)
	{
		let (user_id, count, read_receipt) = r?;
		*max_edu_count = cmp::max(count, *max_edu_count);

		if user_id.server_name() != services().globals.server_name() {
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

async fn send_events(dest: Destination, events: Vec<SendingEventType>) -> Result<Destination, (Destination, Error)> {
	match dest {
		Destination::Normal(ref server) => send_events_dest_normal(&dest, server, events).await,
		Destination::Appservice(ref id) => send_events_dest_appservice(&dest, id, events).await,
		Destination::Push(ref userid, ref pushkey) => send_events_dest_push(&dest, userid, pushkey, events).await,
	}
}

#[tracing::instrument(skip(dest, events))]
async fn send_events_dest_appservice(
	dest: &Destination, id: &String, events: Vec<SendingEventType>,
) -> Result<Destination, (Destination, Error)> {
	let mut pdu_jsons = Vec::new();

	for event in &events {
		match event {
			SendingEventType::Pdu(pdu_id) => {
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
			SendingEventType::Edu(_) | SendingEventType::Flush => {
				// Appservices don't need EDUs (?) and flush only;
				// no new content
			},
		}
	}

	let permit = services().sending.maximum_requests.acquire().await;

	let response = match appservice::send_request(
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
						SendingEventType::Edu(b) | SendingEventType::Pdu(b) => &**b,
						SendingEventType::Flush => &[],
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

	drop(permit);

	response
}

#[tracing::instrument(skip(dest, events))]
async fn send_events_dest_push(
	dest: &Destination, userid: &OwnedUserId, pushkey: &String, events: Vec<SendingEventType>,
) -> Result<Destination, (Destination, Error)> {
	let mut pdus = Vec::new();

	for event in &events {
		match event {
			SendingEventType::Pdu(pdu_id) => {
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
			SendingEventType::Edu(_) | SendingEventType::Flush => {
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

		let permit = services().sending.maximum_requests.acquire().await;

		let _response = services()
			.pusher
			.send_push_notice(userid, unread, &pusher, rules_for_user, &pdu)
			.await
			.map(|_response| dest.clone())
			.map_err(|e| (dest.clone(), e));

		drop(permit);
	}

	Ok(dest.clone())
}

#[tracing::instrument(skip(dest, events), name = "")]
async fn send_events_dest_normal(
	dest: &Destination, server_name: &OwnedServerName, events: Vec<SendingEventType>,
) -> Result<Destination, (Destination, Error)> {
	let mut edu_jsons = Vec::new();
	let mut pdu_jsons = Vec::new();

	for event in &events {
		match event {
			SendingEventType::Pdu(pdu_id) => {
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
			SendingEventType::Edu(edu) => {
				if let Ok(raw) = serde_json::from_slice(edu) {
					edu_jsons.push(raw);
				}
			},
			SendingEventType::Flush => {
				// flush only; no new content
			},
		}
	}

	let permit = services().sending.maximum_requests.acquire().await;
	let client = &services().globals.client.sender;
	let response = send::send_request(
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
						SendingEventType::Edu(b) | SendingEventType::Pdu(b) => &**b,
						SendingEventType::Flush => &[],
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
	.map_err(|e| (dest.clone(), e));

	drop(permit);

	response
}

impl Destination {
	#[tracing::instrument(skip(self))]
	pub(crate) fn get_prefix(&self) -> Vec<u8> {
		let mut prefix = match self {
			Destination::Appservice(server) => {
				let mut p = b"+".to_vec();
				p.extend_from_slice(server.as_bytes());
				p
			},
			Destination::Push(user, pushkey) => {
				let mut p = b"$".to_vec();
				p.extend_from_slice(user.as_bytes());
				p.push(0xFF);
				p.extend_from_slice(pushkey.as_bytes());
				p
			},
			Destination::Normal(server) => {
				let mut p = Vec::new();
				p.extend_from_slice(server.as_bytes());
				p
			},
		};
		prefix.push(0xFF);

		prefix
	}
}
