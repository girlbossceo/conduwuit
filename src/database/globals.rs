use crate::{utils, Result};

pub const COUNTER: &str = "c";

pub struct Globals {
    pub(super) globals: sled::Tree,
    server_name: String,
    keypair: ruma::signatures::Ed25519KeyPair,
    reqwest_client: reqwest::Client,
}

impl Globals {
    pub fn load(globals: sled::Tree, server_name: String) -> Self {
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
            server_name,
            keypair,
            reqwest_client: reqwest::Client::new(),
        }
    }

    /// Returns the server_name of the server.
    pub fn server_name(&self) -> &str {
        &self.server_name
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
}
