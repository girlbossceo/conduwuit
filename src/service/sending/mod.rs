mod appservice;
mod data;
mod dest;
mod send;
mod sender;

use std::{fmt::Debug, sync::Arc};

use async_trait::async_trait;
use conduit::{err, utils::ReadyExt, warn, Result, Server};
use futures::{future::ready, Stream, StreamExt, TryStreamExt};
use ruma::{
	api::{appservice::Registration, OutgoingRequest},
	RoomId, ServerName, UserId,
};
use tokio::sync::Mutex;

use self::data::Data;
pub use self::dest::Destination;
use crate::{account_data, client, globals, presence, pusher, resolver, rooms, server_keys, users, Dep};

pub struct Service {
	server: Arc<Server>,
	services: Services,
	pub db: Data,
	sender: loole::Sender<Msg>,
	receiver: Mutex<loole::Receiver<Msg>>,
}

struct Services {
	client: Dep<client::Service>,
	globals: Dep<globals::Service>,
	resolver: Dep<resolver::Service>,
	state: Dep<rooms::state::Service>,
	state_cache: Dep<rooms::state_cache::Service>,
	user: Dep<rooms::user::Service>,
	users: Dep<users::Service>,
	presence: Dep<presence::Service>,
	read_receipt: Dep<rooms::read_receipt::Service>,
	timeline: Dep<rooms::timeline::Service>,
	account_data: Dep<account_data::Service>,
	appservice: Dep<crate::appservice::Service>,
	pusher: Dep<pusher::Service>,
	server_keys: Dep<server_keys::Service>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Msg {
	dest: Destination,
	event: SendingEvent,
	queue_id: Vec<u8>,
}

#[allow(clippy::module_name_repetitions)]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum SendingEvent {
	Pdu(Vec<u8>), // pduid
	Edu(Vec<u8>), // pdu json
	Flush,        // none
}

#[async_trait]
impl crate::Service for Service {
	fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
		let (sender, receiver) = loole::unbounded();
		Ok(Arc::new(Self {
			server: args.server.clone(),
			services: Services {
				client: args.depend::<client::Service>("client"),
				globals: args.depend::<globals::Service>("globals"),
				resolver: args.depend::<resolver::Service>("resolver"),
				state: args.depend::<rooms::state::Service>("rooms::state"),
				state_cache: args.depend::<rooms::state_cache::Service>("rooms::state_cache"),
				user: args.depend::<rooms::user::Service>("rooms::user"),
				users: args.depend::<users::Service>("users"),
				presence: args.depend::<presence::Service>("presence"),
				read_receipt: args.depend::<rooms::read_receipt::Service>("rooms::read_receipt"),
				timeline: args.depend::<rooms::timeline::Service>("rooms::timeline"),
				account_data: args.depend::<account_data::Service>("account_data"),
				appservice: args.depend::<crate::appservice::Service>("appservice"),
				pusher: args.depend::<pusher::Service>("pusher"),
				server_keys: args.depend::<server_keys::Service>("server_keys"),
			},
			db: Data::new(&args),
			sender,
			receiver: Mutex::new(receiver),
		}))
	}

	async fn worker(self: Arc<Self>) -> Result<()> {
		// trait impl can't be split between files so this just glues to mod sender
		self.sender().await
	}

	fn interrupt(&self) {
		if !self.sender.is_closed() {
			self.sender.close();
		}
	}

	fn name(&self) -> &str { crate::service::make_name(std::module_path!()) }
}

impl Service {
	#[tracing::instrument(skip(self, pdu_id, user, pushkey), level = "debug")]
	pub fn send_pdu_push(&self, pdu_id: &[u8], user: &UserId, pushkey: String) -> Result<()> {
		let dest = Destination::Push(user.to_owned(), pushkey);
		let event = SendingEvent::Pdu(pdu_id.to_owned());
		let _cork = self.db.db.cork();
		let keys = self.db.queue_requests(&[(&event, &dest)]);
		self.dispatch(Msg {
			dest,
			event,
			queue_id: keys.into_iter().next().expect("request queue key"),
		})
	}

	#[tracing::instrument(skip(self), level = "debug")]
	pub fn send_pdu_appservice(&self, appservice_id: String, pdu_id: Vec<u8>) -> Result<()> {
		let dest = Destination::Appservice(appservice_id);
		let event = SendingEvent::Pdu(pdu_id);
		let _cork = self.db.db.cork();
		let keys = self.db.queue_requests(&[(&event, &dest)]);
		self.dispatch(Msg {
			dest,
			event,
			queue_id: keys.into_iter().next().expect("request queue key"),
		})
	}

	#[tracing::instrument(skip(self, room_id, pdu_id), level = "debug")]
	pub async fn send_pdu_room(&self, room_id: &RoomId, pdu_id: &[u8]) -> Result<()> {
		let servers = self
			.services
			.state_cache
			.room_servers(room_id)
			.ready_filter(|server_name| !self.services.globals.server_is_ours(server_name));

		self.send_pdu_servers(servers, pdu_id).await
	}

	#[tracing::instrument(skip(self, servers, pdu_id), level = "debug")]
	pub async fn send_pdu_servers<'a, S>(&self, servers: S, pdu_id: &[u8]) -> Result<()>
	where
		S: Stream<Item = &'a ServerName> + Send + 'a,
	{
		let _cork = self.db.db.cork();
		let requests = servers
			.map(|server| (Destination::Normal(server.into()), SendingEvent::Pdu(pdu_id.into())))
			.collect::<Vec<_>>()
			.await;

		let keys = self
			.db
			.queue_requests(&requests.iter().map(|(o, e)| (e, o)).collect::<Vec<_>>());

		for ((dest, event), queue_id) in requests.into_iter().zip(keys) {
			self.dispatch(Msg {
				dest,
				event,
				queue_id,
			})?;
		}

		Ok(())
	}

	#[tracing::instrument(skip(self, server, serialized), level = "debug")]
	pub fn send_edu_server(&self, server: &ServerName, serialized: Vec<u8>) -> Result<()> {
		let dest = Destination::Normal(server.to_owned());
		let event = SendingEvent::Edu(serialized);
		let _cork = self.db.db.cork();
		let keys = self.db.queue_requests(&[(&event, &dest)]);
		self.dispatch(Msg {
			dest,
			event,
			queue_id: keys.into_iter().next().expect("request queue key"),
		})
	}

	#[tracing::instrument(skip(self, room_id, serialized), level = "debug")]
	pub async fn send_edu_room(&self, room_id: &RoomId, serialized: Vec<u8>) -> Result<()> {
		let servers = self
			.services
			.state_cache
			.room_servers(room_id)
			.ready_filter(|server_name| !self.services.globals.server_is_ours(server_name));

		self.send_edu_servers(servers, serialized).await
	}

	#[tracing::instrument(skip(self, servers, serialized), level = "debug")]
	pub async fn send_edu_servers<'a, S>(&self, servers: S, serialized: Vec<u8>) -> Result<()>
	where
		S: Stream<Item = &'a ServerName> + Send + 'a,
	{
		let _cork = self.db.db.cork();
		let requests = servers
			.map(|server| (Destination::Normal(server.to_owned()), SendingEvent::Edu(serialized.clone())))
			.collect::<Vec<_>>()
			.await;

		let keys = self
			.db
			.queue_requests(&requests.iter().map(|(o, e)| (e, o)).collect::<Vec<_>>());

		for ((dest, event), queue_id) in requests.into_iter().zip(keys) {
			self.dispatch(Msg {
				dest,
				event,
				queue_id,
			})?;
		}

		Ok(())
	}

	#[tracing::instrument(skip(self, room_id), level = "debug")]
	pub async fn flush_room(&self, room_id: &RoomId) -> Result<()> {
		let servers = self
			.services
			.state_cache
			.room_servers(room_id)
			.ready_filter(|server_name| !self.services.globals.server_is_ours(server_name));

		self.flush_servers(servers).await
	}

	#[tracing::instrument(skip(self, servers), level = "debug")]
	pub async fn flush_servers<'a, S>(&self, servers: S) -> Result<()>
	where
		S: Stream<Item = &'a ServerName> + Send + 'a,
	{
		servers
			.map(ToOwned::to_owned)
			.map(Destination::Normal)
			.map(Ok)
			.try_for_each(|dest| {
				ready(self.dispatch(Msg {
					dest,
					event: SendingEvent::Flush,
					queue_id: Vec::<u8>::new(),
				}))
			})
			.await
	}

	#[tracing::instrument(skip_all, name = "request")]
	pub async fn send_federation_request<T>(&self, dest: &ServerName, request: T) -> Result<T::IncomingResponse>
	where
		T: OutgoingRequest + Debug + Send,
	{
		let client = &self.services.client.federation;
		self.send(client, dest, request).await
	}

	/// Sends a request to an appservice
	///
	/// Only returns None if there is no url specified in the appservice
	/// registration file
	pub async fn send_appservice_request<T>(
		&self, registration: Registration, request: T,
	) -> Result<Option<T::IncomingResponse>>
	where
		T: OutgoingRequest + Debug + Send,
	{
		let client = &self.services.client.appservice;
		appservice::send_request(client, registration, request).await
	}

	/// Cleanup event data
	/// Used for instance after we remove an appservice registration
	#[tracing::instrument(skip(self), level = "debug")]
	pub async fn cleanup_events(&self, appservice_id: String) {
		self.db
			.delete_all_requests_for(&Destination::Appservice(appservice_id))
			.await;
	}

	fn dispatch(&self, msg: Msg) -> Result<()> {
		debug_assert!(!self.sender.is_full(), "channel full");
		debug_assert!(!self.sender.is_closed(), "channel closed");
		self.sender.send(msg).map_err(|e| err!("{e}"))
	}
}
