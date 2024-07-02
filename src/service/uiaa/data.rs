use std::{
	collections::BTreeMap,
	sync::{Arc, RwLock},
};

use conduit::{Error, Result};
use database::{Database, Map};
use ruma::{
	api::client::{error::ErrorKind, uiaa::UiaaInfo},
	CanonicalJsonValue, DeviceId, OwnedDeviceId, OwnedUserId, UserId,
};

pub struct Data {
	userdevicesessionid_uiaarequest: RwLock<BTreeMap<(OwnedUserId, OwnedDeviceId, String), CanonicalJsonValue>>,
	userdevicesessionid_uiaainfo: Arc<Map>,
}

impl Data {
	pub(super) fn new(db: &Arc<Database>) -> Self {
		Self {
			userdevicesessionid_uiaarequest: RwLock::new(BTreeMap::new()),
			userdevicesessionid_uiaainfo: db["userdevicesessionid_uiaainfo"].clone(),
		}
	}

	pub(super) fn set_uiaa_request(
		&self, user_id: &UserId, device_id: &DeviceId, session: &str, request: &CanonicalJsonValue,
	) -> Result<()> {
		self.userdevicesessionid_uiaarequest
			.write()
			.unwrap()
			.insert(
				(user_id.to_owned(), device_id.to_owned(), session.to_owned()),
				request.to_owned(),
			);

		Ok(())
	}

	pub(super) fn get_uiaa_request(
		&self, user_id: &UserId, device_id: &DeviceId, session: &str,
	) -> Option<CanonicalJsonValue> {
		self.userdevicesessionid_uiaarequest
			.read()
			.unwrap()
			.get(&(user_id.to_owned(), device_id.to_owned(), session.to_owned()))
			.map(ToOwned::to_owned)
	}

	pub(super) fn update_uiaa_session(
		&self, user_id: &UserId, device_id: &DeviceId, session: &str, uiaainfo: Option<&UiaaInfo>,
	) -> Result<()> {
		let mut userdevicesessionid = user_id.as_bytes().to_vec();
		userdevicesessionid.push(0xFF);
		userdevicesessionid.extend_from_slice(device_id.as_bytes());
		userdevicesessionid.push(0xFF);
		userdevicesessionid.extend_from_slice(session.as_bytes());

		if let Some(uiaainfo) = uiaainfo {
			self.userdevicesessionid_uiaainfo.insert(
				&userdevicesessionid,
				&serde_json::to_vec(&uiaainfo).expect("UiaaInfo::to_vec always works"),
			)?;
		} else {
			self.userdevicesessionid_uiaainfo
				.remove(&userdevicesessionid)?;
		}

		Ok(())
	}

	pub(super) fn get_uiaa_session(&self, user_id: &UserId, device_id: &DeviceId, session: &str) -> Result<UiaaInfo> {
		let mut userdevicesessionid = user_id.as_bytes().to_vec();
		userdevicesessionid.push(0xFF);
		userdevicesessionid.extend_from_slice(device_id.as_bytes());
		userdevicesessionid.push(0xFF);
		userdevicesessionid.extend_from_slice(session.as_bytes());

		serde_json::from_slice(
			&self
				.userdevicesessionid_uiaainfo
				.get(&userdevicesessionid)?
				.ok_or(Error::BadRequest(ErrorKind::forbidden(), "UIAA session does not exist."))?,
		)
		.map_err(|_| Error::bad_database("UiaaInfo in userdeviceid_uiaainfo is invalid."))
	}
}
