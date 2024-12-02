use std::{convert, pin::Pin, sync::Arc};

use conduit::Result;
use futures::{
	stream::FusedStream,
	task::{Context, Poll},
	Stream,
};
use rocksdb::{ColumnFamily, ReadOptions};

use super::{slice_longevity, Cursor, From, State};
use crate::{keyval::Key, Engine};

pub(crate) struct KeysRev<'a> {
	state: State<'a>,
}

impl<'a> KeysRev<'a> {
	pub(crate) fn new(db: &'a Arc<Engine>, cf: &'a Arc<ColumnFamily>, opts: ReadOptions) -> Self {
		Self {
			state: State::new(db, cf, opts),
		}
	}
}

impl<'a> convert::From<State<'a>> for KeysRev<'a> {
	fn from(state: State<'a>) -> Self {
		Self {
			state,
		}
	}
}

impl<'a> Cursor<'a, Key<'a>> for KeysRev<'a> {
	fn state(&self) -> &State<'a> { &self.state }

	#[inline]
	fn fetch(&self) -> Option<Key<'a>> { self.state.fetch_key().map(slice_longevity) }

	#[inline]
	fn seek(&mut self) { self.state.seek_rev(); }

	#[inline]
	fn init(self, from: From<'a>) -> Self {
		Self {
			state: self.state.init_rev(from),
		}
	}
}

impl<'a> Stream for KeysRev<'a> {
	type Item = Result<Key<'a>>;

	fn poll_next(mut self: Pin<&mut Self>, _ctx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
		Poll::Ready(self.seek_and_get())
	}
}

impl FusedStream for KeysRev<'_> {
	#[inline]
	fn is_terminated(&self) -> bool { !self.state.init && !self.state.valid() }
}
