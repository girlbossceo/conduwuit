use std::fmt::Debug;

use conduit::implement;
use ruma::{OwnedServerName, OwnedUserId};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Destination {
	Appservice(String),
	Push(OwnedUserId, String), // user and pushkey
	Normal(OwnedServerName),
}

#[implement(Destination)]
#[must_use]
pub(super) fn get_prefix(&self) -> Vec<u8> {
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
