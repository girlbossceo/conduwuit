pub struct OwnedKeyVal(pub OwnedKey, pub OwnedVal);
pub(crate) type OwnedKeyValPair = (OwnedKey, OwnedVal);
pub type OwnedVal = Vec<Byte>;
pub type OwnedKey = Vec<Byte>;

pub struct KeyVal<'item>(pub &'item Key, pub &'item Val);
pub(crate) type KeyValPair<'item> = (&'item Key, &'item Val);
pub type Val = [Byte];
pub type Key = [Byte];

pub(crate) type Byte = u8;

impl OwnedKeyVal {
	#[inline]
	#[must_use]
	pub fn as_slice(&self) -> KeyVal<'_> { KeyVal(&self.0, &self.1) }

	#[inline]
	#[must_use]
	pub fn to_tuple(self) -> OwnedKeyValPair { (self.0, self.1) }
}

impl From<OwnedKeyValPair> for OwnedKeyVal {
	#[inline]
	fn from((key, val): OwnedKeyValPair) -> Self { Self(key, val) }
}

impl From<&KeyVal<'_>> for OwnedKeyVal {
	fn from(slice: &KeyVal<'_>) -> Self { slice.to_owned() }
}

impl From<KeyValPair<'_>> for OwnedKeyVal {
	fn from((key, val): KeyValPair<'_>) -> Self { Self(Vec::from(key), Vec::from(val)) }
}

impl From<OwnedKeyVal> for OwnedKeyValPair {
	#[inline]
	fn from(val: OwnedKeyVal) -> Self { val.to_tuple() }
}

impl KeyVal<'_> {
	#[inline]
	#[must_use]
	pub fn to_owned(&self) -> OwnedKeyVal { OwnedKeyVal::from(self) }

	#[inline]
	#[must_use]
	pub fn as_tuple(&self) -> KeyValPair<'_> { (self.0, self.1) }
}

impl<'a> From<&'a OwnedKeyVal> for KeyVal<'a> {
	#[inline]
	fn from(owned: &'a OwnedKeyVal) -> Self { owned.as_slice() }
}

impl<'a> From<&'a OwnedKeyValPair> for KeyVal<'a> {
	#[inline]
	fn from((key, val): &'a OwnedKeyValPair) -> Self { KeyVal(key.as_slice(), val.as_slice()) }
}

impl<'a> From<KeyValPair<'a>> for KeyVal<'a> {
	#[inline]
	fn from((key, val): KeyValPair<'a>) -> Self { KeyVal(key, val) }
}
