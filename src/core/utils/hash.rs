mod argon;
mod sha256;

use crate::Result;

pub fn password(password: &str) -> Result<String> { argon::password(password) }

pub fn verify_password(password: &str, password_hash: &str) -> Result<()> {
	argon::verify_password(password, password_hash)
}

#[must_use]
pub fn calculate_hash(keys: &[&[u8]]) -> Vec<u8> { sha256::hash(keys) }
