use crate::{utils, Error, Result};
use js_int::UInt;
use log::debug;
use ruma::{
    api::client::{
        error::ErrorKind,
        r0::{
            device::Device,
            keys::{AlgorithmAndDeviceId, DeviceKeys, KeyAlgorithm, OneTimeKey},
            uiaa::{AuthData, AuthFlow, UiaaInfo, UiaaResponse},
        },
    },
    events::{to_device::AnyToDeviceEvent, EventJson, EventType},
    identifiers::{DeviceId, UserId},
};
use serde_json::value::RawValue;
use std::{collections::BTreeMap, convert::TryFrom, time::SystemTime};

pub struct Uiaa {
    pub(super) userdeviceid_uiaainfo: sled::Tree, // User-interactive authentication
}

impl Uiaa {
    /// Creates a new Uiaa session. Make sure the session token is unique.
    pub fn create(&self, user_id: &UserId, device_id: &str, uiaainfo: &UiaaInfo) -> Result<()> {
        self.update_uiaa_session(user_id, device_id, Some(uiaainfo))
    }

    pub fn try_auth(
        &self,
        user_id: &UserId,
        device_id: &DeviceId,
        auth: &AuthData,
        uiaainfo: &UiaaInfo,
        users: &super::users::Users,
        globals: &super::globals::Globals,
    ) -> Result<(bool, UiaaInfo)> {
        if let AuthData::DirectRequest {
            kind,
            session,
            auth_parameters,
        } = &auth
        {
            let mut uiaainfo = session
                .as_ref()
                .map(|session| {
                    Ok::<_, Error>(self.get_uiaa_session(&user_id, &"".to_owned(), session)?)
                })
                .unwrap_or(Ok(uiaainfo.clone()))?;

            // Find out what the user completed
            match &**kind {
                "m.login.password" => {
                    if auth_parameters["identifier"]["type"] != "m.id.user" {
                        panic!("identifier not supported");
                    }

                    let user_id = UserId::parse_with_server_name(
                        auth_parameters["identifier"]["user"].as_str().unwrap(),
                        globals.server_name(),
                    )?;
                    let password = auth_parameters["password"].as_str().unwrap();

                    // Check if password is correct
                    if let Some(hash) = users.password_hash(&user_id)? {
                        let hash_matches =
                            argon2::verify_encoded(&hash, password.as_bytes()).unwrap_or(false);

                        if !hash_matches {
                            debug!("Invalid password.");
                            uiaainfo.auth_error = Some(ruma::api::client::error::ErrorBody {
                                kind: ErrorKind::Forbidden,
                                message: "Invalid username or password.".to_owned(),
                            });
                            return Ok((false, uiaainfo));
                        }
                    }

                    // Password was correct! Let's add it to `completed`
                    uiaainfo.completed.push("m.login.password".to_owned());
                }
                "m.login.dummy" => {
                    uiaainfo.completed.push("m.login.dummy".to_owned());
                }
                k => panic!("type not supported: {}", k),
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
                self.update_uiaa_session(user_id, device_id, Some(&uiaainfo))?;
                return Ok((false, uiaainfo));
            }

            // UIAA was successful! Remove this session and return true
            self.update_uiaa_session(user_id, device_id, None)?;
            return Ok((true, uiaainfo));
        } else {
            panic!("FallbackAcknowledgement is not supported yet");
        }
    }

    fn update_uiaa_session(
        &self,
        user_id: &UserId,
        device_id: &str,
        uiaainfo: Option<&UiaaInfo>,
    ) -> Result<()> {
        let mut userdeviceid = user_id.to_string().as_bytes().to_vec();
        userdeviceid.push(0xff);
        userdeviceid.extend_from_slice(device_id.as_bytes());

        if let Some(uiaainfo) = uiaainfo {
            self.userdeviceid_uiaainfo
                .insert(&userdeviceid, &*serde_json::to_string(&uiaainfo)?)?;
        } else {
            self.userdeviceid_uiaainfo.remove(&userdeviceid)?;
        }

        Ok(())
    }

    fn get_uiaa_session(
        &self,
        user_id: &UserId,
        device_id: &str,
        session: &str,
    ) -> Result<UiaaInfo> {
        let mut userdeviceid = user_id.to_string().as_bytes().to_vec();
        userdeviceid.push(0xff);
        userdeviceid.extend_from_slice(device_id.as_bytes());

        let uiaainfo = serde_json::from_slice::<UiaaInfo>(
            &self
                .userdeviceid_uiaainfo
                .get(&userdeviceid)?
                .ok_or(Error::BadRequest("session does not exist"))?,
        )?;

        if uiaainfo
            .session
            .as_ref()
            .filter(|&s| s == session)
            .is_none()
        {
            return Err(Error::BadRequest("wrong session token"));
        }

        Ok(uiaainfo)
    }
}
