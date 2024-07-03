mod argon;
mod sha256;

use crate::Result;

#[inline]
pub fn password(password: &str) -> Result<String> { argon::password(password) }

#[inline]
pub fn verify_password(password: &str, password_hash: &str) -> Result<()> {
	argon::verify_password(password, password_hash)
}

#[inline]
pub fn calculate_hash(keys: &[&[u8]]) -> Vec<u8> { sha256::hash(keys) }
