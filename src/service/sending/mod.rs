mod appservice;
mod data;
mod send;
mod sender;

use std::{fmt::Debug, sync::Arc};

use async_trait::async_trait;
use conduit::{err, warn, Result, Server};
use ruma::{
	api::{appservice::Registration, OutgoingRequest},
	OwnedServerName, OwnedUserId, RoomId, ServerName, UserId,
};
use tokio::sync::Mutex;

use crate::{account_data, client, globals, presence, pusher, resolver, rooms, server_is_ours, users, Dep};

pub struct Service {
	server: Arc<Server>,
	services: Services,
	pub db: data::Data,
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
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Msg {
	dest: Destination,
	event: SendingEvent,
	queue_id: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Destination {
	Appservice(String),
	Push(OwnedUserId, String), // user and pushkey
	Normal(OwnedServerName),
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
			},
			db: data::Data::new(&args),
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
		let keys = self.db.queue_requests(&[(&dest, event.clone())])?;
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
		let keys = self.db.queue_requests(&[(&dest, event.clone())])?;
		self.dispatch(Msg {
			dest,
			event,
			queue_id: keys.into_iter().next().expect("request queue key"),
		})
	}

	#[tracing::instrument(skip(self, room_id, pdu_id), level = "debug")]
	pub fn send_pdu_room(&self, room_id: &RoomId, pdu_id: &[u8]) -> Result<()> {
		let servers = self
			.services
			.state_cache
			.room_servers(room_id)
			.filter_map(Result::ok)
			.filter(|server_name| !server_is_ours(server_name));

		self.send_pdu_servers(servers, pdu_id)
	}

	#[tracing::instrument(skip(self, servers, pdu_id), level = "debug")]
	pub fn send_pdu_servers<I: Iterator<Item = OwnedServerName>>(&self, servers: I, pdu_id: &[u8]) -> Result<()> {
		let requests = servers
			.into_iter()
			.map(|server| (Destination::Normal(server), SendingEvent::Pdu(pdu_id.to_owned())))
			.collect::<Vec<_>>();
		let _cork = self.db.db.cork();
		let keys = self.db.queue_requests(
			&requests
				.iter()
				.map(|(o, e)| (o, e.clone()))
				.collect::<Vec<_>>(),
		)?;
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
		let keys = self.db.queue_requests(&[(&dest, event.clone())])?;
		self.dispatch(Msg {
			dest,
			event,
			queue_id: keys.into_iter().next().expect("request queue key"),
		})
	}

	#[tracing::instrument(skip(self, room_id, serialized), level = "debug")]
	pub fn send_edu_room(&self, room_id: &RoomId, serialized: Vec<u8>) -> Result<()> {
		let servers = self
			.services
			.state_cache
			.room_servers(room_id)
			.filter_map(Result::ok)
			.filter(|server_name| !server_is_ours(server_name));

		self.send_edu_servers(servers, serialized)
	}

	#[tracing::instrument(skip(self, servers, serialized), level = "debug")]
	pub fn send_edu_servers<I: Iterator<Item = OwnedServerName>>(&self, servers: I, serialized: Vec<u8>) -> Result<()> {
		let requests = servers
			.into_iter()
			.map(|server| (Destination::Normal(server), SendingEvent::Edu(serialized.clone())))
			.collect::<Vec<_>>();
		let _cork = self.db.db.cork();
		let keys = self.db.queue_requests(
			&requests
				.iter()
				.map(|(o, e)| (o, e.clone()))
				.collect::<Vec<_>>(),
		)?;

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
	pub fn flush_room(&self, room_id: &RoomId) -> Result<()> {
		let servers = self
			.services
			.state_cache
			.room_servers(room_id)
			.filter_map(Result::ok)
			.filter(|server_name| !server_is_ours(server_name));

		self.flush_servers(servers)
	}

	#[tracing::instrument(skip(self, servers), level = "debug")]
	pub fn flush_servers<I: Iterator<Item = OwnedServerName>>(&self, servers: I) -> Result<()> {
		let requests = servers.into_iter().map(Destination::Normal);
		for dest in requests {
			self.dispatch(Msg {
				dest,
				event: SendingEvent::Flush,
				queue_id: Vec::<u8>::new(),
			})?;
		}

		Ok(())
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
	pub fn cleanup_events(&self, appservice_id: String) -> Result<()> {
		self.db
			.delete_all_requests_for(&Destination::Appservice(appservice_id))?;

		Ok(())
	}

	fn dispatch(&self, msg: Msg) -> Result<()> {
		debug_assert!(!self.sender.is_full(), "channel full");
		debug_assert!(!self.sender.is_closed(), "channel closed");
		self.sender.send(msg).map_err(|e| err!("{e}"))
	}
}

impl Destination {
	#[must_use]
	pub fn get_prefix(&self) -> Vec<u8> {
		match self {
			Self::Normal(server) => {
				let len = server.as_bytes().len().saturating_add(1);

				let mut p = Vec::with_capacity(len);
				p.extend_from_slice(server.as_bytes());
				p.push(0xFF);
				p
			},
			Self::Appservice(server) => {
				let sigil = b"+";
				let len = sigil
					.len()
					.saturating_add(server.as_bytes().len())
					.saturating_add(1);

				let mut p = Vec::with_capacity(len);
				p.extend_from_slice(sigil);
				p.extend_from_slice(server.as_bytes());
				p.push(0xFF);
				p
			},
			Self::Push(user, pushkey) => {
				let sigil = b"$";
				let len = sigil
					.len()
					.saturating_add(user.as_bytes().len())
					.saturating_add(1)
					.saturating_add(pushkey.as_bytes().len())
					.saturating_add(1);

				let mut p = Vec::with_capacity(len);
				p.extend_from_slice(sigil);
				p.extend_from_slice(user.as_bytes());
				p.push(0xFF);
				p.extend_from_slice(pushkey.as_bytes());
				p.push(0xFF);
				p
			},
		}
	}
}
