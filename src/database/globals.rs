use crate::{utils, Result};

pub const COUNTER: &str = "c";

pub struct Globals {
    pub(super) globals: sled::Tree,
    keypair: ruma::signatures::Ed25519KeyPair,
    reqwest_client: reqwest::Client,
    server_name: String,
    registration_disabled: bool,
}

impl Globals {
    pub fn load(globals: sled::Tree, config: &rocket::Config) -> Self {
        let keypair = ruma::signatures::Ed25519KeyPair::new(
            &*globals
                .update_and_fetch("keypair", utils::generate_keypair)
                .unwrap()
                .unwrap(),
            "key1".to_owned(),
        )
        .unwrap();

        Self {
            globals,
            keypair,
            reqwest_client: reqwest::Client::new(),
            server_name: config
                .get_str("server_name")
                .unwrap_or("localhost")
                .to_owned(),
            registration_disabled: config.get_bool("registration_disabled").unwrap_or(false),
        }
    }

    /// Returns this server's keypair.
    pub fn keypair(&self) -> &ruma::signatures::Ed25519KeyPair {
        &self.keypair
    }

    /// Returns a reqwest client which can be used to send requests.
    pub fn reqwest_client(&self) -> &reqwest::Client {
        &self.reqwest_client
    }

    pub fn next_count(&self) -> Result<u64> {
        Ok(utils::u64_from_bytes(
            &self
                .globals
                .update_and_fetch(COUNTER, utils::increment)?
                .expect("utils::increment will always put in a value"),
        ))
    }

    pub fn current_count(&self) -> Result<u64> {
        Ok(self
            .globals
            .get(COUNTER)?
            .map_or(0_u64, |bytes| utils::u64_from_bytes(&bytes)))
    }

    pub fn server_name(&self) -> &str {
        &self.server_name
    }

    pub fn registration_disabled(&self) -> bool {
        self.registration_disabled
    }
}
