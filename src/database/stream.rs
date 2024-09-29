mod items;
mod items_rev;
mod keys;
mod keys_rev;

use std::sync::Arc;

use conduit::{utils::exchange, Error, Result};
use rocksdb::{ColumnFamily, DBRawIteratorWithThreadMode, ReadOptions};

pub(crate) use self::{items::Items, items_rev::ItemsRev, keys::Keys, keys_rev::KeysRev};
use crate::{
	engine::Db,
	keyval::{Key, KeyVal, Val},
	util::map_err,
	Engine, Slice,
};

struct State<'a> {
	inner: Inner<'a>,
	seek: bool,
	init: bool,
}

trait Cursor<'a, T> {
	fn state(&self) -> &State<'a>;

	fn fetch(&self) -> Option<T>;

	fn seek(&mut self);

	fn get(&self) -> Option<Result<T>> {
		self.fetch()
			.map(Ok)
			.or_else(|| self.state().status().map(Err))
	}

	fn seek_and_get(&mut self) -> Option<Result<T>> {
		self.seek();
		self.get()
	}
}

type Inner<'a> = DBRawIteratorWithThreadMode<'a, Db>;
type From<'a> = Option<Key<'a>>;

impl<'a> State<'a> {
	fn new(db: &'a Arc<Engine>, cf: &'a Arc<ColumnFamily>, opts: ReadOptions) -> Self {
		Self {
			inner: db.db.raw_iterator_cf_opt(&**cf, opts),
			init: true,
			seek: false,
		}
	}

	fn init_fwd(mut self, from: From<'_>) -> Self {
		if let Some(key) = from {
			self.inner.seek(key);
			self.seek = true;
		}

		self
	}

	fn init_rev(mut self, from: From<'_>) -> Self {
		if let Some(key) = from {
			self.inner.seek_for_prev(key);
			self.seek = true;
		}

		self
	}

	fn seek_fwd(&mut self) {
		if !exchange(&mut self.init, false) {
			self.inner.next();
		} else if !self.seek {
			self.inner.seek_to_first();
		}
	}

	fn seek_rev(&mut self) {
		if !exchange(&mut self.init, false) {
			self.inner.prev();
		} else if !self.seek {
			self.inner.seek_to_last();
		}
	}

	fn fetch_key(&self) -> Option<Key<'_>> { self.inner.key().map(Key::from) }

	fn _fetch_val(&self) -> Option<Val<'_>> { self.inner.value().map(Val::from) }

	fn fetch(&self) -> Option<KeyVal<'_>> { self.inner.item().map(KeyVal::from) }

	fn status(&self) -> Option<Error> { self.inner.status().map_err(map_err).err() }

	fn valid(&self) -> bool { self.inner.valid() }
}

fn keyval_longevity<'a, 'b: 'a>(item: KeyVal<'a>) -> KeyVal<'b> {
	(slice_longevity::<'a, 'b>(item.0), slice_longevity::<'a, 'b>(item.1))
}

fn slice_longevity<'a, 'b: 'a>(item: &'a Slice) -> &'b Slice {
	// SAFETY: The lifetime of the data returned by the rocksdb cursor is only valid
	// between each movement of the cursor. It is hereby unsafely extended to match
	// the lifetime of the cursor itself. This is due to the limitation of the
	// Stream trait where the Item is incapable of conveying a lifetime; this is due
	// to GAT's being unstable during its development. This unsafety can be removed
	// as soon as this limitation is addressed by an upcoming version.
	//
	// We have done our best to mitigate the implications of this in conjunction
	// with the deserialization API such that borrows being held across movements of
	// the cursor do not happen accidentally. The compiler will still error when
	// values herein produced try to leave a closure passed to a StreamExt API. But
	// escapes can happen if you explicitly and intentionally attempt it, and there
	// will be no compiler error or warning. This is primarily the case with
	// calling collect() without a preceding map(ToOwned::to_owned). A collection
	// of references here is illegal, but this will not be enforced by the compiler.
	unsafe { std::mem::transmute(item) }
}