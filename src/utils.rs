use std::time::{SystemTime, UNIX_EPOCH};

pub fn millis_since_unix_epoch() -> js_int::UInt {
    (SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u32)
        .into()
}
