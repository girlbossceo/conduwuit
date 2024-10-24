use std::{pin::Pin, sync::Arc};

use conduit::Result;
use futures::{
	stream::FusedStream,
	task::{Context, Poll},
	Stream,
};
use rocksdb::{ColumnFamily, ReadOptions};

use super::{keyval_longevity, Cursor, From, State};
use crate::{keyval::KeyVal, Engine};

pub(crate) struct ItemsRev<'a> {
	state: State<'a>,
}

impl<'a> ItemsRev<'a> {
	pub(crate) fn new(db: &'a Arc<Engine>, cf: &'a Arc<ColumnFamily>, opts: ReadOptions, from: From<'_>) -> Self {
		Self {
			state: State::new(db, cf, opts).init_rev(from),
		}
	}
}

impl<'a> Cursor<'a, KeyVal<'a>> for ItemsRev<'a> {
	fn state(&self) -> &State<'a> { &self.state }

	fn fetch(&self) -> Option<KeyVal<'a>> { self.state.fetch().map(keyval_longevity) }

	fn seek(&mut self) { self.state.seek_rev(); }
}

impl<'a> Stream for ItemsRev<'a> {
	type Item = Result<KeyVal<'a>>;

	fn poll_next(mut self: Pin<&mut Self>, _ctx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
		Poll::Ready(self.seek_and_get())
	}
}

impl FusedStream for ItemsRev<'_> {
	fn is_terminated(&self) -> bool { !self.state.init && !self.state.valid() }
}
