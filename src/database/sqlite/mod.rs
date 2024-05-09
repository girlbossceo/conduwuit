use std::{
	cell::RefCell,
	future::Future,
	path::{Path, PathBuf},
	pin::Pin,
	sync::Arc,
};

use conduit::{Config, Result};
use parking_lot::{Mutex, MutexGuard};
use rusqlite::{Connection, DatabaseName::Main, OptionalExtension};
use thread_local::ThreadLocal;
use tracing::debug;

use super::{watchers::Watchers, KeyValueDatabaseEngine, KvTree};

thread_local! {
	static READ_CONNECTION: RefCell<Option<&'static Connection>> = const { RefCell::new(None) };
	static READ_CONNECTION_ITERATOR: RefCell<Option<&'static Connection>> = const { RefCell::new(None) };
}

struct PreparedStatementIterator<'a> {
	iterator: Box<dyn Iterator<Item = TupleOfBytes> + 'a>,
	_statement_ref: AliasableBox<rusqlite::Statement<'a>>,
}

impl Iterator for PreparedStatementIterator<'_> {
	type Item = TupleOfBytes;

	fn next(&mut self) -> Option<Self::Item> { self.iterator.next() }
}

struct AliasableBox<T>(*mut T);
impl<T> Drop for AliasableBox<T> {
	fn drop(&mut self) {
		// SAFETY: This is cursed and relies on non-local reasoning.
		//
		// In order for this to be safe:
		//
		// * All aliased references to this value must have been dropped first, for
		//   example by coming after its referrers in struct fields, because struct
		//   fields are automatically dropped in order from top to bottom in the absence
		//   of an explicit Drop impl. Otherwise, the referrers may read into
		//   deallocated memory.
		// * This type must not be copyable or cloneable. Otherwise, double-free can
		//   occur.
		//
		// These conditions are met, but again, note that changing safe code in
		// this module can result in unsoundness if any of these constraints are
		// violated.
		unsafe { drop(Box::from_raw(self.0)) }
	}
}

pub(crate) struct Engine {
	writer: Mutex<Connection>,
	read_conn_tls: ThreadLocal<Connection>,
	read_iterator_conn_tls: ThreadLocal<Connection>,

	path: PathBuf,
	cache_size_per_thread: u32,
}

impl Engine {
	fn prepare_conn(path: &Path, cache_size_kb: u32) -> Result<Connection> {
		let conn = Connection::open(path)?;

		conn.pragma_update(Some(Main), "page_size", 2048)?;
		conn.pragma_update(Some(Main), "journal_mode", "WAL")?;
		conn.pragma_update(Some(Main), "synchronous", "NORMAL")?;
		conn.pragma_update(Some(Main), "cache_size", -i64::from(cache_size_kb))?;
		conn.pragma_update(Some(Main), "wal_autocheckpoint", 0)?;

		Ok(conn)
	}

	fn write_lock(&self) -> MutexGuard<'_, Connection> { self.writer.lock() }

	fn read_lock(&self) -> &Connection {
		self.read_conn_tls
			.get_or(|| Self::prepare_conn(&self.path, self.cache_size_per_thread).unwrap())
	}

	fn read_lock_iterator(&self) -> &Connection {
		self.read_iterator_conn_tls
			.get_or(|| Self::prepare_conn(&self.path, self.cache_size_per_thread).unwrap())
	}

	fn flush_wal(self: &Arc<Self>) -> Result<()> {
		self.write_lock()
			.pragma_update(Some(Main), "wal_checkpoint", "RESTART")?;
		Ok(())
	}
}

impl KeyValueDatabaseEngine for Arc<Engine> {
	fn open(config: &Config) -> Result<Self> {
		let path = Path::new(&config.database_path).join("conduit.db");

		// calculates cache-size per permanent connection
		// 1. convert MB to KiB
		// 2. divide by permanent connections + permanent iter connections + write
		//    connection
		// 3. round down to nearest integer
		#[allow(
			clippy::as_conversions,
			clippy::cast_possible_truncation,
			clippy::cast_precision_loss,
			clippy::cast_sign_loss
		)]
		let cache_size_per_thread = ((config.db_cache_capacity_mb * 1024.0) / ((num_cpus::get() as f64 * 2.0) + 1.0)) as u32;

		let writer = Mutex::new(Engine::prepare_conn(&path, cache_size_per_thread)?);

		let arc = Arc::new(Engine {
			writer,
			read_conn_tls: ThreadLocal::new(),
			read_iterator_conn_tls: ThreadLocal::new(),
			path,
			cache_size_per_thread,
		});

		Ok(arc)
	}

	fn open_tree(&self, name: &str) -> Result<Arc<dyn KvTree>> {
		self.write_lock().execute(
			&format!("CREATE TABLE IF NOT EXISTS {name} ( \"key\" BLOB PRIMARY KEY, \"value\" BLOB NOT NULL )"),
			[],
		)?;

		Ok(Arc::new(SqliteTable {
			engine: Arc::clone(self),
			name: name.to_owned(),
			watchers: Watchers::default(),
		}))
	}

	fn flush(&self) -> Result<()> {
		// we enabled PRAGMA synchronous=normal, so this should not be necessary
		Ok(())
	}

	fn cleanup(&self) -> Result<()> { self.flush_wal() }
}

struct SqliteTable {
	engine: Arc<Engine>,
	name: String,
	watchers: Watchers,
}

type TupleOfBytes = (Vec<u8>, Vec<u8>);

impl SqliteTable {
	fn get_with_guard(&self, guard: &Connection, key: &[u8]) -> Result<Option<Vec<u8>>> {
		Ok(guard
			.prepare(format!("SELECT value FROM {} WHERE key = ?", self.name).as_str())?
			.query_row([key], |row| row.get(0))
			.optional()?)
	}

	fn insert_with_guard(&self, guard: &Connection, key: &[u8], value: &[u8]) -> Result<()> {
		guard.execute(
			format!("INSERT OR REPLACE INTO {} (key, value) VALUES (?, ?)", self.name).as_str(),
			[key, value],
		)?;
		Ok(())
	}

	fn iter_with_guard<'a>(&'a self, guard: &'a Connection) -> Box<dyn Iterator<Item = TupleOfBytes> + 'a> {
		let statement = Box::leak(Box::new(
			guard
				.prepare(&format!("SELECT key, value FROM {} ORDER BY key ASC", &self.name))
				.unwrap(),
		));

		let statement_ref = AliasableBox(statement);

		//let name = self.name.clone();

		let iterator = Box::new(
			statement
				.query_map([], |row| Ok((row.get_unwrap(0), row.get_unwrap(1))))
				.unwrap()
				.map(Result::unwrap),
		);

		Box::new(PreparedStatementIterator {
			iterator,
			_statement_ref: statement_ref,
		})
	}
}

impl KvTree for SqliteTable {
	fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> { self.get_with_guard(self.engine.read_lock(), key) }

	fn insert(&self, key: &[u8], value: &[u8]) -> Result<()> {
		let guard = self.engine.write_lock();
		self.insert_with_guard(&guard, key, value)?;
		drop(guard);
		self.watchers.wake(key);
		Ok(())
	}

	fn insert_batch(&self, iter: &mut dyn Iterator<Item = (Vec<u8>, Vec<u8>)>) -> Result<()> {
		let guard = self.engine.write_lock();

		guard.execute("BEGIN", [])?;
		for (key, value) in iter {
			self.insert_with_guard(&guard, &key, &value)?;
		}
		guard.execute("COMMIT", [])?;

		drop(guard);

		Ok(())
	}

	fn increment_batch(&self, iter: &mut dyn Iterator<Item = Vec<u8>>) -> Result<()> {
		let guard = self.engine.write_lock();

		guard.execute("BEGIN", [])?;
		for key in iter {
			let old = self.get_with_guard(&guard, &key)?;
			let new = conduit::utils::increment(old.as_deref());
			self.insert_with_guard(&guard, &key, &new)?;
		}
		guard.execute("COMMIT", [])?;

		drop(guard);

		Ok(())
	}

	fn remove(&self, key: &[u8]) -> Result<()> {
		let guard = self.engine.write_lock();

		guard.execute(format!("DELETE FROM {} WHERE key = ?", self.name).as_str(), [key])?;

		Ok(())
	}

	fn iter<'a>(&'a self) -> Box<dyn Iterator<Item = TupleOfBytes> + 'a> {
		let guard = self.engine.read_lock_iterator();

		self.iter_with_guard(guard)
	}

	fn iter_from<'a>(&'a self, from: &[u8], backwards: bool) -> Box<dyn Iterator<Item = TupleOfBytes> + 'a> {
		let guard = self.engine.read_lock_iterator();
		let from = from.to_vec(); // TODO change interface?

		//let name = self.name.clone();

		if backwards {
			let statement = Box::leak(Box::new(
				guard
					.prepare(&format!(
						"SELECT key, value FROM {} WHERE key <= ? ORDER BY key DESC",
						&self.name
					))
					.unwrap(),
			));

			let statement_ref = AliasableBox(statement);

			let iterator = Box::new(
				statement
					.query_map([from], |row| Ok((row.get_unwrap(0), row.get_unwrap(1))))
					.unwrap()
					.map(Result::unwrap),
			);
			Box::new(PreparedStatementIterator {
				iterator,
				_statement_ref: statement_ref,
			})
		} else {
			let statement = Box::leak(Box::new(
				guard
					.prepare(&format!(
						"SELECT key, value FROM {} WHERE key >= ? ORDER BY key ASC",
						&self.name
					))
					.unwrap(),
			));

			let statement_ref = AliasableBox(statement);

			let iterator = Box::new(
				statement
					.query_map([from], |row| Ok((row.get_unwrap(0), row.get_unwrap(1))))
					.unwrap()
					.map(Result::unwrap),
			);

			Box::new(PreparedStatementIterator {
				iterator,
				_statement_ref: statement_ref,
			})
		}
	}

	fn increment(&self, key: &[u8]) -> Result<Vec<u8>> {
		let guard = self.engine.write_lock();

		let old = self.get_with_guard(&guard, key)?;

		let new = conduit::utils::increment(old.as_deref());

		self.insert_with_guard(&guard, key, &new)?;

		Ok(new)
	}

	fn scan_prefix<'a>(&'a self, prefix: Vec<u8>) -> Box<dyn Iterator<Item = TupleOfBytes> + 'a> {
		Box::new(
			self.iter_from(&prefix, false)
				.take_while(move |(key, _)| key.starts_with(&prefix)),
		)
	}

	fn watch_prefix<'a>(&'a self, prefix: &[u8]) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
		self.watchers.watch(prefix)
	}

	fn clear(&self) -> Result<()> {
		debug!("clear: running");
		self.engine
			.write_lock()
			.execute(format!("DELETE FROM {}", self.name).as_str(), [])?;
		debug!("clear: ran");
		Ok(())
	}
}
