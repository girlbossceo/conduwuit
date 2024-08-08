use std::sync::Arc;

use conduit::{
	utils,
	utils::{stream::TryIgnore, ReadyExt},
	Error, Result,
};
use database::{Database, Deserialized, Map};
use futures::{Stream, StreamExt};
use ruma::{ServerName, UserId};

use super::{Destination, SendingEvent};
use crate::{globals, Dep};

pub(super) type OutgoingItem = (Key, SendingEvent, Destination);
pub(super) type SendingItem = (Key, SendingEvent);
pub(super) type QueueItem = (Key, SendingEvent);
pub(super) type Key = Vec<u8>;

pub struct Data {
	servercurrentevent_data: Arc<Map>,
	servernameevent_data: Arc<Map>,
	servername_educount: Arc<Map>,
	pub(super) db: Arc<Database>,
	services: Services,
}

struct Services {
	globals: Dep<globals::Service>,
}

impl Data {
	pub(super) fn new(args: &crate::Args<'_>) -> Self {
		let db = &args.db;
		Self {
			servercurrentevent_data: db["servercurrentevent_data"].clone(),
			servernameevent_data: db["servernameevent_data"].clone(),
			servername_educount: db["servername_educount"].clone(),
			db: args.db.clone(),
			services: Services {
				globals: args.depend::<globals::Service>("globals"),
			},
		}
	}

	pub(super) fn delete_active_request(&self, key: &[u8]) { self.servercurrentevent_data.remove(key); }

	pub(super) async fn delete_all_active_requests_for(&self, destination: &Destination) {
		let prefix = destination.get_prefix();
		self.servercurrentevent_data
			.raw_keys_prefix(&prefix)
			.ignore_err()
			.ready_for_each(|key| self.servercurrentevent_data.remove(key))
			.await;
	}

	pub(super) async fn delete_all_requests_for(&self, destination: &Destination) {
		let prefix = destination.get_prefix();
		self.servercurrentevent_data
			.raw_keys_prefix(&prefix)
			.ignore_err()
			.ready_for_each(|key| self.servercurrentevent_data.remove(key))
			.await;

		self.servernameevent_data
			.raw_keys_prefix(&prefix)
			.ignore_err()
			.ready_for_each(|key| self.servernameevent_data.remove(key))
			.await;
	}

	pub(super) fn mark_as_active(&self, events: &[QueueItem]) {
		for (key, e) in events {
			if key.is_empty() {
				continue;
			}

			let value = if let SendingEvent::Edu(value) = &e {
				&**value
			} else {
				&[]
			};
			self.servercurrentevent_data.insert(key, value);
			self.servernameevent_data.remove(key);
		}
	}

	#[inline]
	pub fn active_requests(&self) -> impl Stream<Item = OutgoingItem> + Send + '_ {
		self.servercurrentevent_data
			.raw_stream()
			.ignore_err()
			.map(|(key, val)| {
				let (dest, event) = parse_servercurrentevent(key, val).expect("invalid servercurrentevent");

				(key.to_vec(), event, dest)
			})
	}

	#[inline]
	pub fn active_requests_for<'a>(&'a self, destination: &Destination) -> impl Stream<Item = SendingItem> + Send + 'a {
		let prefix = destination.get_prefix();
		self.servercurrentevent_data
			.stream_raw_prefix(&prefix)
			.ignore_err()
			.map(|(key, val)| {
				let (_, event) = parse_servercurrentevent(key, val).expect("invalid servercurrentevent");

				(key.to_vec(), event)
			})
	}

	pub(super) fn queue_requests(&self, requests: &[(&SendingEvent, &Destination)]) -> Vec<Vec<u8>> {
		let mut batch = Vec::new();
		let mut keys = Vec::new();
		for (event, destination) in requests {
			let mut key = destination.get_prefix();
			if let SendingEvent::Pdu(value) = &event {
				key.extend_from_slice(value);
			} else {
				key.extend_from_slice(&self.services.globals.next_count().unwrap().to_be_bytes());
			}
			let value = if let SendingEvent::Edu(value) = &event {
				&**value
			} else {
				&[]
			};
			batch.push((key.clone(), value.to_owned()));
			keys.push(key);
		}

		self.servernameevent_data.insert_batch(batch.iter());
		keys
	}

	pub fn queued_requests<'a>(&'a self, destination: &Destination) -> impl Stream<Item = QueueItem> + Send + 'a {
		let prefix = destination.get_prefix();
		self.servernameevent_data
			.stream_raw_prefix(&prefix)
			.ignore_err()
			.map(|(key, val)| {
				let (_, event) = parse_servercurrentevent(key, val).expect("invalid servercurrentevent");

				(key.to_vec(), event)
			})
	}

	pub(super) fn set_latest_educount(&self, server_name: &ServerName, last_count: u64) {
		self.servername_educount
			.insert(server_name.as_bytes(), &last_count.to_be_bytes());
	}

	pub async fn get_latest_educount(&self, server_name: &ServerName) -> u64 {
		self.servername_educount
			.qry(server_name)
			.await
			.deserialized()
			.unwrap_or(0)
	}
}

#[tracing::instrument(skip(key), level = "debug")]
fn parse_servercurrentevent(key: &[u8], value: &[u8]) -> Result<(Destination, SendingEvent)> {
	// Appservices start with a plus
	Ok::<_, Error>(if key.starts_with(b"+") {
		let mut parts = key[1..].splitn(2, |&b| b == 0xFF);

		let server = parts.next().expect("splitn always returns one element");
		let event = parts
			.next()
			.ok_or_else(|| Error::bad_database("Invalid bytes in servercurrentpdus."))?;

		let server = utils::string_from_bytes(server)
			.map_err(|_| Error::bad_database("Invalid server bytes in server_currenttransaction"))?;

		(
			Destination::Appservice(server),
			if value.is_empty() {
				SendingEvent::Pdu(event.to_vec())
			} else {
				SendingEvent::Edu(value.to_vec())
			},
		)
	} else if key.starts_with(b"$") {
		let mut parts = key[1..].splitn(3, |&b| b == 0xFF);

		let user = parts.next().expect("splitn always returns one element");
		let user_string = utils::string_from_bytes(user)
			.map_err(|_| Error::bad_database("Invalid user string in servercurrentevent"))?;
		let user_id =
			UserId::parse(user_string).map_err(|_| Error::bad_database("Invalid user id in servercurrentevent"))?;

		let pushkey = parts
			.next()
			.ok_or_else(|| Error::bad_database("Invalid bytes in servercurrentpdus."))?;
		let pushkey_string = utils::string_from_bytes(pushkey)
			.map_err(|_| Error::bad_database("Invalid pushkey in servercurrentevent"))?;

		let event = parts
			.next()
			.ok_or_else(|| Error::bad_database("Invalid bytes in servercurrentpdus."))?;

		(
			Destination::Push(user_id, pushkey_string),
			if value.is_empty() {
				SendingEvent::Pdu(event.to_vec())
			} else {
				// I'm pretty sure this should never be called
				SendingEvent::Edu(value.to_vec())
			},
		)
	} else {
		let mut parts = key.splitn(2, |&b| b == 0xFF);

		let server = parts.next().expect("splitn always returns one element");
		let event = parts
			.next()
			.ok_or_else(|| Error::bad_database("Invalid bytes in servercurrentpdus."))?;

		let server = utils::string_from_bytes(server)
			.map_err(|_| Error::bad_database("Invalid server bytes in server_currenttransaction"))?;

		(
			Destination::Normal(
				ServerName::parse(server)
					.map_err(|_| Error::bad_database("Invalid server string in server_currenttransaction"))?,
			),
			if value.is_empty() {
				SendingEvent::Pdu(event.to_vec())
			} else {
				SendingEvent::Edu(value.to_vec())
			},
		)
	})
}
