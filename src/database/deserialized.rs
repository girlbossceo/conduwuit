use std::convert::identity;

use conduit::Result;
use serde::Deserialize;

pub trait Deserialized {
	fn map_de<T, U, F>(self, f: F) -> Result<U>
	where
		F: FnOnce(T) -> U,
		T: for<'de> Deserialize<'de>;

	#[inline]
	fn deserialized<T>(self) -> Result<T>
	where
		T: for<'de> Deserialize<'de>,
		Self: Sized,
	{
		self.map_de(identity::<T>)
	}
}
