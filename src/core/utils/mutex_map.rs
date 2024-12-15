use std::{fmt::Debug, hash::Hash, sync::Arc};

use tokio::sync::OwnedMutexGuard as Omg;

/// Map of Mutexes
pub struct MutexMap<Key, Val> {
	map: Map<Key, Val>,
}

pub struct Guard<Key, Val> {
	map: Map<Key, Val>,
	val: Omg<Val>,
}

type Map<Key, Val> = Arc<MapMutex<Key, Val>>;
type MapMutex<Key, Val> = std::sync::Mutex<HashMap<Key, Val>>;
type HashMap<Key, Val> = std::collections::HashMap<Key, Value<Val>>;
type Value<Val> = Arc<tokio::sync::Mutex<Val>>;

impl<Key, Val> MutexMap<Key, Val>
where
	Key: Send + Hash + Eq + Clone,
	Val: Send + Default,
{
	#[must_use]
	pub fn new() -> Self {
		Self {
			map: Map::new(MapMutex::new(HashMap::new())),
		}
	}

	#[tracing::instrument(skip(self), level = "debug")]
	pub async fn lock<K>(&self, k: &K) -> Guard<Key, Val>
	where
		K: ?Sized + Send + Sync + Debug,
		Key: for<'a> From<&'a K>,
	{
		let val = self
			.map
			.lock()
			.expect("locked")
			.entry(k.into())
			.or_default()
			.clone();

		Guard::<Key, Val> {
			map: Arc::clone(&self.map),
			val: val.lock_owned().await,
		}
	}

	#[must_use]
	pub fn contains(&self, k: &Key) -> bool { self.map.lock().expect("locked").contains_key(k) }

	#[must_use]
	pub fn is_empty(&self) -> bool { self.map.lock().expect("locked").is_empty() }

	#[must_use]
	pub fn len(&self) -> usize { self.map.lock().expect("locked").len() }
}

impl<Key, Val> Default for MutexMap<Key, Val>
where
	Key: Send + Hash + Eq + Clone,
	Val: Send + Default,
{
	fn default() -> Self { Self::new() }
}

impl<Key, Val> Drop for Guard<Key, Val> {
	fn drop(&mut self) {
		if Arc::strong_count(Omg::mutex(&self.val)) <= 2 {
			self.map.lock().expect("locked").retain(|_, val| {
				!Arc::ptr_eq(val, Omg::mutex(&self.val)) || Arc::strong_count(val) > 2
			});
		}
	}
}
