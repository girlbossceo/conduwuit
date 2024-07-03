use crate::Result;

#[inline]
#[must_use]
pub fn increment(old: Option<&[u8]>) -> [u8; 8] {
	old.map(TryInto::try_into)
		.map_or(0_u64, |val| val.map_or(0_u64, u64::from_be_bytes))
		.wrapping_add(1)
		.to_be_bytes()
}

/// Parses the big-endian bytes into an u64.
#[inline]
pub fn u64_from_bytes(bytes: &[u8]) -> Result<u64> {
	let array: [u8; 8] = bytes.try_into()?;
	Ok(u64_from_u8x8(array))
}

/// Parses the 8 big-endian bytes into an u64.
#[inline]
#[must_use]
pub fn u64_from_u8(bytes: &[u8]) -> u64 {
	let bytes: &[u8; 8] = bytes.try_into().expect("must slice at least 8 bytes");
	u64_from_u8x8(*bytes)
}

#[inline]
#[must_use]
pub fn u64_from_u8x8(bytes: [u8; 8]) -> u64 { u64::from_be_bytes(bytes) }
