use std::time::{SystemTime, UNIX_EPOCH};

pub fn millis_since_unix_epoch() -> js_int::UInt {
    (SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u32)
        .into()
}

pub fn bytes_to_string(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).expect("convert bytes to string")
}

pub fn vec_to_bytes(vec: Vec<String>) -> Vec<u8> {
    vec.into_iter()
        .map(|string| string.into_bytes())
        .collect::<Vec<Vec<u8>>>()
        .join(&0)
}

pub fn bytes_to_vec(bytes: &[u8]) -> Vec<String> {
    bytes
        .split(|&b| b == 0)
        .map(|bytes_string| bytes_to_string(bytes_string))
        .collect::<Vec<String>>()
}
