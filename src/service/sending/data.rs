use std::sync::Arc;

use conduit::{utils, Error, Result};
use database::{Database, Map};
use ruma::{ServerName, UserId};

use super::{Destination, SendingEvent};
use crate::{globals, Dep};

type OutgoingSendingIter<'a> = Box<dyn Iterator<Item = Result<(Vec<u8>, Destination, SendingEvent)>> + 'a>;
type SendingEventIter<'a> = Box<dyn Iterator<Item = Result<(Vec<u8>, SendingEvent)>> + 'a>;

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

	#[inline]
	pub fn active_requests(&self) -> OutgoingSendingIter<'_> {
		Box::new(
			self.servercurrentevent_data
				.iter()
				.map(|(key, v)| parse_servercurrentevent(&key, v).map(|(k, e)| (key, k, e))),
		)
	}

	#[inline]
	pub fn active_requests_for<'a>(&'a self, destination: &Destination) -> SendingEventIter<'a> {
		let prefix = destination.get_prefix();
		Box::new(
			self.servercurrentevent_data
				.scan_prefix(prefix)
				.map(|(key, v)| parse_servercurrentevent(&key, v).map(|(_, e)| (key, e))),
		)
	}

	pub(super) fn delete_active_request(&self, key: &[u8]) -> Result<()> { self.servercurrentevent_data.remove(key) }

	pub(super) fn delete_all_active_requests_for(&self, destination: &Destination) -> Result<()> {
		let prefix = destination.get_prefix();
		for (key, _) in self.servercurrentevent_data.scan_prefix(prefix) {
			self.servercurrentevent_data.remove(&key)?;
		}

		Ok(())
	}

	pub(super) fn delete_all_requests_for(&self, destination: &Destination) -> Result<()> {
		let prefix = destination.get_prefix();
		for (key, _) in self.servercurrentevent_data.scan_prefix(prefix.clone()) {
			self.servercurrentevent_data.remove(&key).unwrap();
		}

		for (key, _) in self.servernameevent_data.scan_prefix(prefix) {
			self.servernameevent_data.remove(&key).unwrap();
		}

		Ok(())
	}

	pub(super) fn queue_requests(&self, requests: &[(&Destination, SendingEvent)]) -> Result<Vec<Vec<u8>>> {
		let mut batch = Vec::new();
		let mut keys = Vec::new();
		for (destination, event) in requests {
			let mut key = destination.get_prefix();
			if let SendingEvent::Pdu(value) = &event {
				key.extend_from_slice(value);
			} else {
				key.extend_from_slice(&self.services.globals.next_count()?.to_be_bytes());
			}
			let value = if let SendingEvent::Edu(value) = &event {
				&**value
			} else {
				&[]
			};
			batch.push((key.clone(), value.to_owned()));
			keys.push(key);
		}
		self.servernameevent_data
			.insert_batch(batch.iter().map(database::KeyVal::from))?;
		Ok(keys)
	}

	pub fn queued_requests<'a>(
		&'a self, destination: &Destination,
	) -> Box<dyn Iterator<Item = Result<(SendingEvent, Vec<u8>)>> + 'a> {
		let prefix = destination.get_prefix();
		return Box::new(
			self.servernameevent_data
				.scan_prefix(prefix)
				.map(|(k, v)| parse_servercurrentevent(&k, v).map(|(_, ev)| (ev, k))),
		);
	}

	pub(super) fn mark_as_active(&self, events: &[(SendingEvent, Vec<u8>)]) -> Result<()> {
		for (e, key) in events {
			if key.is_empty() {
				continue;
			}

			let value = if let SendingEvent::Edu(value) = &e {
				&**value
			} else {
				&[]
			};
			self.servercurrentevent_data.insert(key, value)?;
			self.servernameevent_data.remove(key)?;
		}

		Ok(())
	}

	pub(super) fn set_latest_educount(&self, server_name: &ServerName, last_count: u64) -> Result<()> {
		self.servername_educount
			.insert(server_name.as_bytes(), &last_count.to_be_bytes())
	}

	pub fn get_latest_educount(&self, server_name: &ServerName) -> Result<u64> {
		self.servername_educount
			.get(server_name.as_bytes())?
			.map_or(Ok(0), |bytes| {
				utils::u64_from_bytes(&bytes).map_err(|_| Error::bad_database("Invalid u64 in servername_educount."))
			})
	}
}

#[tracing::instrument(skip(key), level = "debug")]
fn parse_servercurrentevent(key: &[u8], value: Vec<u8>) -> Result<(Destination, SendingEvent)> {
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
				SendingEvent::Edu(value)
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
				SendingEvent::Edu(value)
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
				SendingEvent::Edu(value)
			},
		)
	})
}
