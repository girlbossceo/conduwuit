use std::{hash::Hash, sync::Arc};

type Value<Val> = tokio::sync::Mutex<Val>;
type ArcMutex<Val> = Arc<Value<Val>>;
type HashMap<Key, Val> = std::collections::HashMap<Key, ArcMutex<Val>>;
type MapMutex<Key, Val> = std::sync::Mutex<HashMap<Key, Val>>;
type Map<Key, Val> = MapMutex<Key, Val>;

/// Map of Mutexes
pub struct MutexMap<Key, Val> {
	map: Map<Key, Val>,
}

pub struct Guard<Val> {
	_guard: tokio::sync::OwnedMutexGuard<Val>,
}

impl<Key, Val> MutexMap<Key, Val>
where
	Key: Send + Hash + Eq + Clone,
	Val: Send + Default,
{
	#[must_use]
	pub fn new() -> Self {
		Self {
			map: Map::<Key, Val>::new(HashMap::<Key, Val>::new()),
		}
	}

	pub async fn lock<K>(&self, k: &K) -> Guard<Val>
	where
		K: ?Sized + Send + Sync,
		Key: for<'a> From<&'a K>,
	{
		let val = self
			.map
			.lock()
			.expect("map mutex locked")
			.entry(k.into())
			.or_default()
			.clone();

		let guard = val.lock_owned().await;
		Guard::<Val> {
			_guard: guard,
		}
	}
}

impl<Key, Val> Default for MutexMap<Key, Val>
where
	Key: Send + Hash + Eq + Clone,
	Val: Send + Default,
{
	fn default() -> Self { Self::new() }
}
