use std::sync::Arc;

use conduit::{err, utils::stream::TryIgnore, Result};
use database::{Database, Map};
use futures::Stream;
use ruma::api::appservice::Registration;

pub struct Data {
	id_appserviceregistrations: Arc<Map>,
}

impl Data {
	pub(super) fn new(db: &Arc<Database>) -> Self {
		Self {
			id_appserviceregistrations: db["id_appserviceregistrations"].clone(),
		}
	}

	/// Registers an appservice and returns the ID to the caller
	pub(super) fn register_appservice(&self, yaml: &Registration) -> Result<String> {
		let id = yaml.id.as_str();
		self.id_appserviceregistrations
			.insert(id.as_bytes(), serde_yaml::to_string(&yaml).unwrap().as_bytes());

		Ok(id.to_owned())
	}

	/// Remove an appservice registration
	///
	/// # Arguments
	///
	/// * `service_name` - the name you send to register the service previously
	pub(super) fn unregister_appservice(&self, service_name: &str) -> Result<()> {
		self.id_appserviceregistrations
			.remove(service_name.as_bytes());
		Ok(())
	}

	pub async fn get_registration(&self, id: &str) -> Result<Registration> {
		self.id_appserviceregistrations
			.get(id)
			.await
			.and_then(|ref bytes| serde_yaml::from_slice(bytes).map_err(Into::into))
			.map_err(|e| err!(Database("Invalid appservice {id:?} registration: {e:?}")))
	}

	pub(super) fn iter_ids(&self) -> impl Stream<Item = String> + Send + '_ {
		self.id_appserviceregistrations.keys().ignore_err()
	}
}
