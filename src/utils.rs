use rand::prelude::*;
use std::{
    convert::TryInto,
    time::{SystemTime, UNIX_EPOCH},
};

pub fn millis_since_unix_epoch() -> js_int::UInt {
    (SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64)
        .try_into()
        .expect("time millis are <= MAX_SAFE_UINT")
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
