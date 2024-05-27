use std::sync::Arc;

use conduit::{Error, Result};
use database::{KeyValueDatabase, KvTree};
use ruma::{
	api::client::{error::ErrorKind, uiaa::UiaaInfo},
	CanonicalJsonValue, DeviceId, UserId,
};

pub struct Data {
	userdevicesessionid_uiaainfo: Arc<dyn KvTree>,
	db: Arc<KeyValueDatabase>,
}

impl Data {
	pub(super) fn new(db: &Arc<KeyValueDatabase>) -> Self {
		Self {
			userdevicesessionid_uiaainfo: db.userdevicesessionid_uiaainfo.clone(),
			db: db.clone(),
		}
	}

	pub(super) fn set_uiaa_request(
		&self, user_id: &UserId, device_id: &DeviceId, session: &str, request: &CanonicalJsonValue,
	) -> Result<()> {
		self.db
			.userdevicesessionid_uiaarequest
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
		self.db
			.userdevicesessionid_uiaarequest
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
