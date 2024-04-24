mod data;

use std::collections::BTreeMap;

pub(crate) use data::Data;
use futures_util::Future;
use regex::RegexSet;
use ruma::{
	api::appservice::{Namespace, Registration},
	RoomAliasId, RoomId, UserId,
};
use tokio::sync::RwLock;

use crate::{services, Result};

/// Compiled regular expressions for a namespace
#[derive(Clone, Debug)]
pub(crate) struct NamespaceRegex {
	pub(crate) exclusive: Option<RegexSet>,
	pub(crate) non_exclusive: Option<RegexSet>,
}

impl NamespaceRegex {
	/// Checks if this namespace has rights to a namespace
	pub(crate) fn is_match(&self, heystack: &str) -> bool {
		if self.is_exclusive_match(heystack) {
			return true;
		}

		if let Some(non_exclusive) = &self.non_exclusive {
			if non_exclusive.is_match(heystack) {
				return true;
			}
		}
		false
	}

	/// Checks if this namespace has exlusive rights to a namespace
	pub(crate) fn is_exclusive_match(&self, heystack: &str) -> bool {
		if let Some(exclusive) = &self.exclusive {
			if exclusive.is_match(heystack) {
				return true;
			}
		}
		false
	}
}

impl RegistrationInfo {
	pub(crate) fn is_user_match(&self, user_id: &UserId) -> bool {
		self.users.is_match(user_id.as_str()) || self.registration.sender_localpart == user_id.localpart()
	}

	pub(crate) fn is_exclusive_user_match(&self, user_id: &UserId) -> bool {
		self.users.is_exclusive_match(user_id.as_str()) || self.registration.sender_localpart == user_id.localpart()
	}
}

impl TryFrom<Vec<Namespace>> for NamespaceRegex {
	type Error = regex::Error;

	fn try_from(value: Vec<Namespace>) -> Result<Self, regex::Error> {
		let mut exclusive = vec![];
		let mut non_exclusive = vec![];

		for namespace in value {
			if namespace.exclusive {
				exclusive.push(namespace.regex);
			} else {
				non_exclusive.push(namespace.regex);
			}
		}

		Ok(NamespaceRegex {
			exclusive: if exclusive.is_empty() {
				None
			} else {
				Some(RegexSet::new(exclusive)?)
			},
			non_exclusive: if non_exclusive.is_empty() {
				None
			} else {
				Some(RegexSet::new(non_exclusive)?)
			},
		})
	}
}

/// Appservice registration combined with its compiled regular expressions.
#[derive(Clone, Debug)]
pub(crate) struct RegistrationInfo {
	pub(crate) registration: Registration,
	pub(crate) users: NamespaceRegex,
	pub(crate) aliases: NamespaceRegex,
	pub(crate) rooms: NamespaceRegex,
}

impl TryFrom<Registration> for RegistrationInfo {
	type Error = regex::Error;

	fn try_from(value: Registration) -> Result<RegistrationInfo, regex::Error> {
		Ok(RegistrationInfo {
			users: value.namespaces.users.clone().try_into()?,
			aliases: value.namespaces.aliases.clone().try_into()?,
			rooms: value.namespaces.rooms.clone().try_into()?,
			registration: value,
		})
	}
}

pub(crate) struct Service {
	pub(crate) db: &'static dyn Data,
	registration_info: RwLock<BTreeMap<String, RegistrationInfo>>,
}

impl Service {
	pub(crate) fn build(db: &'static dyn Data) -> Result<Self> {
		let mut registration_info = BTreeMap::new();
		// Inserting registrations into cache
		for appservice in db.all()? {
			registration_info.insert(
				appservice.0,
				appservice
					.1
					.try_into()
					.expect("Should be validated on registration"),
			);
		}

		Ok(Self {
			db,
			registration_info: RwLock::new(registration_info),
		})
	}

	/// Registers an appservice and returns the ID to the caller
	pub(crate) async fn register_appservice(&self, yaml: Registration) -> Result<String> {
		//TODO: Check for collisions between exclusive appservice namespaces
		services()
			.appservice
			.registration_info
			.write()
			.await
			.insert(yaml.id.clone(), yaml.clone().try_into()?);

		self.db.register_appservice(yaml)
	}

	/// Remove an appservice registration
	///
	/// # Arguments
	///
	/// * `service_name` - the name you send to register the service previously
	pub(crate) async fn unregister_appservice(&self, service_name: &str) -> Result<()> {
		// removes the appservice registration info
		services()
			.appservice
			.registration_info
			.write()
			.await
			.remove(service_name)
			.ok_or_else(|| crate::Error::AdminCommand("Appservice not found"))?;

		// remove the appservice from the database
		self.db.unregister_appservice(service_name)?;

		// deletes all active requests for the appservice if there are any so we stop
		// sending to the URL
		services().sending.cleanup_events(service_name.to_owned())?;

		Ok(())
	}

	pub(crate) async fn get_registration(&self, id: &str) -> Option<Registration> {
		self.registration_info
			.read()
			.await
			.get(id)
			.cloned()
			.map(|info| info.registration)
	}

	pub(crate) async fn iter_ids(&self) -> Vec<String> {
		self.registration_info
			.read()
			.await
			.keys()
			.cloned()
			.collect()
	}

	pub(crate) async fn find_from_token(&self, token: &str) -> Option<RegistrationInfo> {
		self.read()
			.await
			.values()
			.find(|info| info.registration.as_token == token)
			.cloned()
	}

	/// Checks if a given user id matches any exclusive appservice regex
	pub(crate) async fn is_exclusive_user_id(&self, user_id: &UserId) -> bool {
		self.read()
			.await
			.values()
			.any(|info| info.is_exclusive_user_match(user_id))
	}

	/// Checks if a given room alias matches any exclusive appservice regex
	pub(crate) async fn is_exclusive_alias(&self, alias: &RoomAliasId) -> bool {
		self.read()
			.await
			.values()
			.any(|info| info.aliases.is_exclusive_match(alias.as_str()))
	}

	/// Checks if a given room id matches any exclusive appservice regex
	///
	/// TODO: use this?
	#[allow(dead_code)]
	pub(crate) async fn is_exclusive_room_id(&self, room_id: &RoomId) -> bool {
		self.read()
			.await
			.values()
			.any(|info| info.rooms.is_exclusive_match(room_id.as_str()))
	}

	pub(crate) fn read(
		&self,
	) -> impl Future<Output = tokio::sync::RwLockReadGuard<'_, BTreeMap<String, RegistrationInfo>>> {
		self.registration_info.read()
	}
}
