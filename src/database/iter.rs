use std::{iter::FusedIterator, sync::Arc};

use conduit::Result;
use rocksdb::{ColumnFamily, DBRawIteratorWithThreadMode, Direction, IteratorMode, ReadOptions};

use crate::{engine::Db, map::KeyVal, result, Engine};

type Cursor<'cursor> = DBRawIteratorWithThreadMode<'cursor, Db>;
type Key<'item> = &'item [u8];
type Val<'item> = &'item [u8];
type Item<'item> = (Key<'item>, Val<'item>);

struct State<'cursor> {
	cursor: Cursor<'cursor>,
	direction: Direction,
	valid: bool,
	init: bool,
}

impl<'cursor> State<'cursor> {
	pub(crate) fn new(
		db: &'cursor Arc<Engine>, cf: &'cursor Arc<ColumnFamily>, opts: ReadOptions, mode: &IteratorMode<'_>,
	) -> Self {
		let mut cursor = db.db.raw_iterator_cf_opt(&**cf, opts);
		let direction = into_direction(mode);
		let valid = seek_init(&mut cursor, mode);
		Self {
			cursor,
			direction,
			valid,
			init: true,
		}
	}
}

pub struct Iter<'cursor> {
	state: State<'cursor>,
}

impl<'cursor> Iter<'cursor> {
	pub(crate) fn new(
		db: &'cursor Arc<Engine>, cf: &'cursor Arc<ColumnFamily>, opts: ReadOptions, mode: &IteratorMode<'_>,
	) -> Self {
		Self {
			state: State::new(db, cf, opts, mode),
		}
	}
}

impl Iterator for Iter<'_> {
	type Item = KeyVal;

	fn next(&mut self) -> Option<Self::Item> {
		if !self.state.init && self.state.valid {
			seek_next(&mut self.state.cursor, self.state.direction);
		} else if self.state.init {
			self.state.init = false;
		}

		self.state.cursor.item().map(into_keyval).or_else(|| {
			when_invalid(&mut self.state).expect("iterator invalidated due to error");
			None
		})
	}
}

impl FusedIterator for Iter<'_> {}

fn when_invalid(state: &mut State<'_>) -> Result<()> {
	state.valid = false;
	result(state.cursor.status())
}

fn seek_next(cursor: &mut Cursor<'_>, direction: Direction) {
	match direction {
		Direction::Forward => cursor.next(),
		Direction::Reverse => cursor.prev(),
	}
}

fn seek_init(cursor: &mut Cursor<'_>, mode: &IteratorMode<'_>) -> bool {
	use Direction::{Forward, Reverse};
	use IteratorMode::{End, From, Start};

	match mode {
		Start => cursor.seek_to_first(),
		End => cursor.seek_to_last(),
		From(key, Forward) => cursor.seek(key),
		From(key, Reverse) => cursor.seek_for_prev(key),
	};

	cursor.valid()
}

fn into_direction(mode: &IteratorMode<'_>) -> Direction {
	use Direction::{Forward, Reverse};
	use IteratorMode::{End, From, Start};

	match mode {
		Start | From(_, Forward) => Forward,
		End | From(_, Reverse) => Reverse,
	}
}

fn into_keyval((key, val): Item<'_>) -> KeyVal { (Vec::from(key), Vec::from(val)) }
