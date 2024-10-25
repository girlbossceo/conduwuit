use std::{pin::Pin, sync::Arc};

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
	pub(crate) fn new(db: &'a Arc<Engine>, cf: &'a Arc<ColumnFamily>, opts: ReadOptions, from: From<'_>) -> Self {
		Self {
			state: State::new(db, cf, opts).init_rev(from),
		}
	}
}

impl<'a> Cursor<'a, Key<'a>> for KeysRev<'a> {
	fn state(&self) -> &State<'a> { &self.state }

	fn fetch(&self) -> Option<Key<'a>> { self.state.fetch_key().map(slice_longevity) }

	fn seek(&mut self) { self.state.seek_rev(); }
}

impl<'a> Stream for KeysRev<'a> {
	type Item = Result<Key<'a>>;

	fn poll_next(mut self: Pin<&mut Self>, _ctx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
		Poll::Ready(self.seek_and_get())
	}
}

impl FusedStream for KeysRev<'_> {
	fn is_terminated(&self) -> bool { !self.state.init && !self.state.valid() }
}
