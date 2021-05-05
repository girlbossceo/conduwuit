use crate::{client_server::SESSION_ID_LENGTH, utils, Error, Result};
use ruma::{
    api::client::{
        error::ErrorKind,
        r0::uiaa::{IncomingAuthData, UiaaInfo},
    },
    signatures::CanonicalJsonValue,
    DeviceId, UserId,
};

#[derive(Clone)]
pub struct Uiaa {
    pub(super) userdevicesessionid_uiaainfo: sled::Tree, // User-interactive authentication
    pub(super) userdevicesessionid_uiaarequest: sled::Tree, // UiaaRequest = canonical json value
}

impl Uiaa {
    /// Creates a new Uiaa session. Make sure the session token is unique.
    pub fn create(
        &self,
        user_id: &UserId,
        device_id: &DeviceId,
        uiaainfo: &UiaaInfo,
        json_body: &CanonicalJsonValue,
    ) -> Result<()> {
        self.set_uiaa_request(
            user_id,
            device_id,
            uiaainfo.session.as_ref().expect("session should be set"), // TODO: better session error handling (why is it optional in ruma?)
            json_body,
        )?;
        self.update_uiaa_session(
            user_id,
            device_id,
            uiaainfo.session.as_ref().expect("session should be set"),
            Some(uiaainfo),
        )
    }

    pub fn try_auth(
        &self,
        user_id: &UserId,
        device_id: &DeviceId,
        auth: &IncomingAuthData,
        uiaainfo: &UiaaInfo,
        users: &super::users::Users,
        globals: &super::globals::Globals,
    ) -> Result<(bool, UiaaInfo)> {
        if let IncomingAuthData::DirectRequest {
            kind,
            session,
            auth_parameters,
        } = &auth
        {
            let mut uiaainfo = session
                .as_ref()
                .map(|session| {
                    Ok::<_, Error>(self.get_uiaa_session(&user_id, &device_id, session)?)
                })
                .unwrap_or_else(|| Ok(uiaainfo.clone()))?;

            if uiaainfo.session.is_none() {
                uiaainfo.session = Some(utils::random_string(SESSION_ID_LENGTH));
            }

            // Find out what the user completed
            match &**kind {
                "m.login.password" => {
                    let identifier = auth_parameters.get("identifier").ok_or(Error::BadRequest(
                        ErrorKind::MissingParam,
                        "m.login.password needs identifier.",
                    ))?;

                    let identifier_type = identifier.get("type").ok_or(Error::BadRequest(
                        ErrorKind::MissingParam,
                        "Identifier needs a type.",
                    ))?;

                    if identifier_type != "m.id.user" {
                        return Err(Error::BadRequest(
                            ErrorKind::Unrecognized,
                            "Identifier type not recognized.",
                        ));
                    }

                    let username = identifier
                        .get("user")
                        .ok_or(Error::BadRequest(
                            ErrorKind::MissingParam,
                            "Identifier needs user field.",
                        ))?
                        .as_str()
                        .ok_or(Error::BadRequest(
                            ErrorKind::BadJson,
                            "User is not a string.",
                        ))?;

                    let user_id = UserId::parse_with_server_name(username, globals.server_name())
                        .map_err(|_| {
                        Error::BadRequest(ErrorKind::InvalidParam, "User ID is invalid.")
                    })?;

                    let password = auth_parameters
                        .get("password")
                        .ok_or(Error::BadRequest(
                            ErrorKind::MissingParam,
                            "Password is missing.",
                        ))?
                        .as_str()
                        .ok_or(Error::BadRequest(
                            ErrorKind::BadJson,
                            "Password is not a string.",
                        ))?;

                    // Check if password is correct
                    if let Some(hash) = users.password_hash(&user_id)? {
                        let hash_matches =
                            argon2::verify_encoded(&hash, password.as_bytes()).unwrap_or(false);

                        if !hash_matches {
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
                self.update_uiaa_session(
                    user_id,
                    device_id,
                    uiaainfo.session.as_ref().expect("session is always set"),
                    Some(&uiaainfo),
                )?;
                return Ok((false, uiaainfo));
            }

            // UIAA was successful! Remove this session and return true
            self.update_uiaa_session(
                user_id,
                device_id,
                uiaainfo.session.as_ref().expect("session is always set"),
                None,
            )?;
            Ok((true, uiaainfo))
        } else {
            panic!("FallbackAcknowledgement is not supported yet");
        }
    }

    fn set_uiaa_request(
        &self,
        user_id: &UserId,
        device_id: &DeviceId,
        session: &str,
        request: &CanonicalJsonValue,
    ) -> Result<()> {
        let mut userdevicesessionid = user_id.as_bytes().to_vec();
        userdevicesessionid.push(0xff);
        userdevicesessionid.extend_from_slice(device_id.as_bytes());
        userdevicesessionid.push(0xff);
        userdevicesessionid.extend_from_slice(session.as_bytes());

        self.userdevicesessionid_uiaarequest.insert(
            &userdevicesessionid,
            &*serde_json::to_string(request).expect("json value to string always works"),
        )?;

        Ok(())
    }

    pub fn get_uiaa_request(
        &self,
        user_id: &UserId,
        device_id: &DeviceId,
        session: &str,
    ) -> Result<Option<CanonicalJsonValue>> {
        let mut userdevicesessionid = user_id.as_bytes().to_vec();
        userdevicesessionid.push(0xff);
        userdevicesessionid.extend_from_slice(device_id.as_bytes());
        userdevicesessionid.push(0xff);
        userdevicesessionid.extend_from_slice(session.as_bytes());

        self.userdevicesessionid_uiaarequest
            .get(&userdevicesessionid)?
            .map_or(Ok(None), |bytes| {
                Ok::<_, Error>(Some(
                    serde_json::from_str::<CanonicalJsonValue>(
                        &utils::string_from_bytes(&bytes).map_err(|_| {
                            Error::bad_database("Invalid uiaa request bytes in db.")
                        })?,
                    )
                    .map_err(|_| Error::bad_database("Invalid uiaa request in db."))?,
                ))
            })
    }

    fn update_uiaa_session(
        &self,
        user_id: &UserId,
        device_id: &DeviceId,
        session: &str,
        uiaainfo: Option<&UiaaInfo>,
    ) -> Result<()> {
        let mut userdevicesessionid = user_id.as_bytes().to_vec();
        userdevicesessionid.push(0xff);
        userdevicesessionid.extend_from_slice(device_id.as_bytes());
        userdevicesessionid.push(0xff);
        userdevicesessionid.extend_from_slice(session.as_bytes());

        if let Some(uiaainfo) = uiaainfo {
            self.userdevicesessionid_uiaainfo.insert(
                &userdevicesessionid,
                &*serde_json::to_string(&uiaainfo).expect("UiaaInfo::to_string always works"),
            )?;
        } else {
            self.userdevicesessionid_uiaainfo
                .remove(&userdevicesessionid)?;
        }

        Ok(())
    }

    fn get_uiaa_session(
        &self,
        user_id: &UserId,
        device_id: &DeviceId,
        session: &str,
    ) -> Result<UiaaInfo> {
        let mut userdevicesessionid = user_id.as_bytes().to_vec();
        userdevicesessionid.push(0xff);
        userdevicesessionid.extend_from_slice(device_id.as_bytes());
        userdevicesessionid.push(0xff);
        userdevicesessionid.extend_from_slice(session.as_bytes());

        let uiaainfo = serde_json::from_slice::<UiaaInfo>(
            &self
                .userdevicesessionid_uiaainfo
                .get(&userdevicesessionid)?
                .ok_or(Error::BadRequest(
                    ErrorKind::Forbidden,
                    "UIAA session does not exist.",
                ))?,
        )
        .map_err(|_| Error::bad_database("UiaaInfo in userdeviceid_uiaainfo is invalid."))?;

        Ok(uiaainfo)
    }
}
