use std::ops::Deref;

use rocksdb::DBPinnableSlice;

pub struct Handle<'a> {
	val: DBPinnableSlice<'a>,
}

impl<'a> From<DBPinnableSlice<'a>> for Handle<'a> {
	fn from(val: DBPinnableSlice<'a>) -> Self {
		Self {
			val,
		}
	}
}

impl Deref for Handle<'_> {
	type Target = [u8];

	#[inline]
	fn deref(&self) -> &Self::Target { &self.val }
}

impl AsRef<[u8]> for Handle<'_> {
	#[inline]
	fn as_ref(&self) -> &[u8] { &self.val }
}
