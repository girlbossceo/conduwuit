use std::{
	collections::BTreeMap,
	sync::{Arc, RwLock},
};

use conduit::{
	err, error, implement, utils,
	utils::{hash, string::EMPTY},
	Error, Result,
};
use database::{Deserialized, Map};
use ruma::{
	api::client::{
		error::ErrorKind,
		uiaa::{AuthData, AuthType, Password, UiaaInfo, UserIdentifier},
	},
	CanonicalJsonValue, DeviceId, OwnedDeviceId, OwnedUserId, UserId,
};

use crate::{globals, users, Dep};

pub struct Service {
	userdevicesessionid_uiaarequest: RwLock<RequestMap>,
	db: Data,
	services: Services,
}

struct Services {
	globals: Dep<globals::Service>,
	users: Dep<users::Service>,
}

struct Data {
	userdevicesessionid_uiaainfo: Arc<Map>,
}

type RequestMap = BTreeMap<RequestKey, CanonicalJsonValue>;
type RequestKey = (OwnedUserId, OwnedDeviceId, String);

pub const SESSION_ID_LENGTH: usize = 32;

impl crate::Service for Service {
	fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
		Ok(Arc::new(Self {
			userdevicesessionid_uiaarequest: RwLock::new(RequestMap::new()),
			db: Data {
				userdevicesessionid_uiaainfo: args.db["userdevicesessionid_uiaainfo"].clone(),
			},
			services: Services {
				globals: args.depend::<globals::Service>("globals"),
				users: args.depend::<users::Service>("users"),
			},
		}))
	}

	fn name(&self) -> &str { crate::service::make_name(std::module_path!()) }
}

/// Creates a new Uiaa session. Make sure the session token is unique.
#[implement(Service)]
pub fn create(&self, user_id: &UserId, device_id: &DeviceId, uiaainfo: &UiaaInfo, json_body: &CanonicalJsonValue) {
	// TODO: better session error handling (why is uiaainfo.session optional in
	// ruma?)
	self.set_uiaa_request(
		user_id,
		device_id,
		uiaainfo.session.as_ref().expect("session should be set"),
		json_body,
	);

	self.update_uiaa_session(
		user_id,
		device_id,
		uiaainfo.session.as_ref().expect("session should be set"),
		Some(uiaainfo),
	);
}

#[implement(Service)]
pub async fn try_auth(
	&self, user_id: &UserId, device_id: &DeviceId, auth: &AuthData, uiaainfo: &UiaaInfo,
) -> Result<(bool, UiaaInfo)> {
	let mut uiaainfo = if let Some(session) = auth.session() {
		self.get_uiaa_session(user_id, device_id, session).await?
	} else {
		uiaainfo.clone()
	};

	if uiaainfo.session.is_none() {
		uiaainfo.session = Some(utils::random_string(SESSION_ID_LENGTH));
	}

	match auth {
		// Find out what the user completed
		AuthData::Password(Password {
			identifier,
			password,
			#[cfg(feature = "element_hacks")]
			user,
			..
		}) => {
			#[cfg(feature = "element_hacks")]
			let username = if let Some(UserIdentifier::UserIdOrLocalpart(username)) = identifier {
				username
			} else if let Some(username) = user {
				username
			} else {
				return Err(Error::BadRequest(ErrorKind::Unrecognized, "Identifier type not recognized."));
			};

			#[cfg(not(feature = "element_hacks"))]
			let Some(UserIdentifier::UserIdOrLocalpart(username)) = identifier
			else {
				return Err(Error::BadRequest(ErrorKind::Unrecognized, "Identifier type not recognized."));
			};

			let user_id = UserId::parse_with_server_name(username.clone(), self.services.globals.server_name())
				.map_err(|_| Error::BadRequest(ErrorKind::InvalidParam, "User ID is invalid."))?;

			// Check if password is correct
			if let Ok(hash) = self.services.users.password_hash(&user_id).await {
				let hash_matches = hash::verify_password(password, &hash).is_ok();
				if !hash_matches {
					uiaainfo.auth_error = Some(ruma::api::client::error::StandardErrorBody {
						kind: ErrorKind::forbidden(),
						message: "Invalid username or password.".to_owned(),
					});
					return Ok((false, uiaainfo));
				}
			}

			// Password was correct! Let's add it to `completed`
			uiaainfo.completed.push(AuthType::Password);
		},
		AuthData::RegistrationToken(t) => {
			if self
				.services
				.globals
				.registration_token
				.as_ref()
				.is_some_and(|reg_token| t.token.trim() == reg_token)
			{
				uiaainfo.completed.push(AuthType::RegistrationToken);
			} else {
				uiaainfo.auth_error = Some(ruma::api::client::error::StandardErrorBody {
					kind: ErrorKind::forbidden(),
					message: "Invalid registration token.".to_owned(),
				});
				return Ok((false, uiaainfo));
			}
		},
		AuthData::Dummy(_) => {
			uiaainfo.completed.push(AuthType::Dummy);
		},
		k => error!("type not supported: {:?}", k),
	}

	// Check if a flow now succeeds
	let mut completed = false;
	'flows: for flow in &mut uiaainfo.flows {
		for stage in &flow.stages {
			if !uiaainfo.completed.contains(stage) {
				continue 'flows;
			}
		}
		// We didn't break, so this flow succeeded!
		completed = true;
	}

	if !completed {
		self.update_uiaa_session(
			user_id,
			device_id,
			uiaainfo.session.as_ref().expect("session is always set"),
			Some(&uiaainfo),
		);

		return Ok((false, uiaainfo));
	}

	// UIAA was successful! Remove this session and return true
	self.update_uiaa_session(
		user_id,
		device_id,
		uiaainfo.session.as_ref().expect("session is always set"),
		None,
	);

	Ok((true, uiaainfo))
}

#[implement(Service)]
fn set_uiaa_request(&self, user_id: &UserId, device_id: &DeviceId, session: &str, request: &CanonicalJsonValue) {
	let key = (user_id.to_owned(), device_id.to_owned(), session.to_owned());
	self.userdevicesessionid_uiaarequest
		.write()
		.expect("locked for writing")
		.insert(key, request.to_owned());
}

#[implement(Service)]
pub fn get_uiaa_request(
	&self, user_id: &UserId, device_id: Option<&DeviceId>, session: &str,
) -> Option<CanonicalJsonValue> {
	let key = (
		user_id.to_owned(),
		device_id.unwrap_or_else(|| EMPTY.into()).to_owned(),
		session.to_owned(),
	);

	self.userdevicesessionid_uiaarequest
		.read()
		.expect("locked for reading")
		.get(&key)
		.cloned()
}

#[implement(Service)]
fn update_uiaa_session(&self, user_id: &UserId, device_id: &DeviceId, session: &str, uiaainfo: Option<&UiaaInfo>) {
	let mut userdevicesessionid = user_id.as_bytes().to_vec();
	userdevicesessionid.push(0xFF);
	userdevicesessionid.extend_from_slice(device_id.as_bytes());
	userdevicesessionid.push(0xFF);
	userdevicesessionid.extend_from_slice(session.as_bytes());

	if let Some(uiaainfo) = uiaainfo {
		self.db.userdevicesessionid_uiaainfo.insert(
			&userdevicesessionid,
			&serde_json::to_vec(&uiaainfo).expect("UiaaInfo::to_vec always works"),
		);
	} else {
		self.db
			.userdevicesessionid_uiaainfo
			.remove(&userdevicesessionid);
	}
}

#[implement(Service)]
async fn get_uiaa_session(&self, user_id: &UserId, device_id: &DeviceId, session: &str) -> Result<UiaaInfo> {
	let key = (user_id, device_id, session);
	self.db
		.userdevicesessionid_uiaainfo
		.qry(&key)
		.await
		.deserialized()
		.map_err(|_| err!(Request(Forbidden("UIAA session does not exist."))))
}
