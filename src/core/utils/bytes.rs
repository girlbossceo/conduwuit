use crate::Result;

#[inline]
#[must_use]
pub fn increment(old: Option<&[u8]>) -> [u8; 8] {
	old.map_or(0_u64, u64_from_bytes_or_zero)
		.wrapping_add(1)
		.to_be_bytes()
}

/// Parses 8 big-endian bytes into an u64; panic on invalid argument
#[inline]
#[must_use]
pub fn u64_from_u8(bytes: &[u8]) -> u64 { u64_from_bytes(bytes).expect("must slice at least 8 bytes") }

/// Parses the big-endian bytes into an u64.
#[inline]
#[must_use]
pub fn u64_from_bytes_or_zero(bytes: &[u8]) -> u64 { u64_from_bytes(bytes).unwrap_or(0) }

/// Parses the big-endian bytes into an u64.
#[inline]
pub fn u64_from_bytes(bytes: &[u8]) -> Result<u64> { Ok(u64_from_u8x8(*u8x8_from_bytes(bytes)?)) }

#[inline]
#[must_use]
pub fn u64_from_u8x8(bytes: [u8; 8]) -> u64 { u64::from_be_bytes(bytes) }

#[inline]
pub fn u8x8_from_bytes(bytes: &[u8]) -> Result<&[u8; 8]> { Ok(bytes.try_into()?) }
