use argon2::{Config, Variant};
use rand::prelude::*;
use std::{
    convert::TryInto,
    time::{SystemTime, UNIX_EPOCH},
};

pub fn millis_since_unix_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

pub fn increment(old: Option<&[u8]>) -> Option<Vec<u8>> {
    let number = match old {
        Some(bytes) => {
            let array: [u8; 8] = bytes.try_into().unwrap();
            let number = u64::from_be_bytes(array);
            number + 1
        }
        None => 0,
    };

    Some(number.to_be_bytes().to_vec())
}

pub fn generate_keypair(old: Option<&[u8]>) -> Option<Vec<u8>> {
    Some(
        /*
        old.map(|s| s.to_vec())
            .unwrap_or_else(|| */
        ruma_signatures::Ed25519KeyPair::generate().unwrap(),
    )
}

pub fn u64_from_bytes(bytes: &[u8]) -> u64 {
    let array: [u8; 8] = bytes.try_into().expect("bytes are valid u64");
    u64::from_be_bytes(array)
}

pub fn string_from_bytes(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).expect("bytes are valid utf8")
}

pub fn random_string(length: usize) -> String {
    thread_rng()
        .sample_iter(&rand::distributions::Alphanumeric)
        .take(length)
        .collect()
}

/// Calculate a new hash for the given password
pub fn calculate_hash(password: &str) -> Result<String, argon2::Error> {
    let hashing_config = Config {
        variant: Variant::Argon2id,
        ..Default::default()
    };

    let salt = random_string(32);
    argon2::hash_encoded(password.as_bytes(), salt.as_bytes(), &hashing_config)
}
