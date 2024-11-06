#![allow(clippy::cast_possible_wrap, clippy::cast_sign_loss, clippy::as_conversions)]

use std::{cmp::Ordering, fmt, fmt::Display, str::FromStr};

use ruma::api::Direction;

use crate::{err, Error, Result};

#[derive(Hash, PartialEq, Eq, Clone, Copy, Debug)]
pub enum PduCount {
	Normal(u64),
	Backfilled(i64),
}

impl PduCount {
	#[inline]
	#[must_use]
	pub fn from_unsigned(unsigned: u64) -> Self { Self::from_signed(unsigned as i64) }

	#[inline]
	#[must_use]
	pub fn from_signed(signed: i64) -> Self {
		match signed {
			i64::MIN..=0 => Self::Backfilled(signed),
			_ => Self::Normal(signed as u64),
		}
	}

	#[inline]
	#[must_use]
	pub fn into_unsigned(self) -> u64 {
		self.debug_assert_valid();
		match self {
			Self::Normal(i) => i,
			Self::Backfilled(i) => i as u64,
		}
	}

	#[inline]
	#[must_use]
	pub fn into_signed(self) -> i64 {
		self.debug_assert_valid();
		match self {
			Self::Normal(i) => i as i64,
			Self::Backfilled(i) => i,
		}
	}

	#[inline]
	#[must_use]
	pub fn into_normal(self) -> Self {
		self.debug_assert_valid();
		match self {
			Self::Normal(i) => Self::Normal(i),
			Self::Backfilled(_) => Self::Normal(0),
		}
	}

	#[inline]
	pub fn checked_inc(self, dir: Direction) -> Result<Self, Error> {
		match dir {
			Direction::Forward => self.checked_add(1),
			Direction::Backward => self.checked_sub(1),
		}
	}

	#[inline]
	pub fn checked_add(self, add: u64) -> Result<Self, Error> {
		Ok(match self {
			Self::Normal(i) => Self::Normal(
				i.checked_add(add)
					.ok_or_else(|| err!(Arithmetic("PduCount::Normal overflow")))?,
			),
			Self::Backfilled(i) => Self::Backfilled(
				i.checked_add(add as i64)
					.ok_or_else(|| err!(Arithmetic("PduCount::Backfilled overflow")))?,
			),
		})
	}

	#[inline]
	pub fn checked_sub(self, sub: u64) -> Result<Self, Error> {
		Ok(match self {
			Self::Normal(i) => Self::Normal(
				i.checked_sub(sub)
					.ok_or_else(|| err!(Arithmetic("PduCount::Normal underflow")))?,
			),
			Self::Backfilled(i) => Self::Backfilled(
				i.checked_sub(sub as i64)
					.ok_or_else(|| err!(Arithmetic("PduCount::Backfilled underflow")))?,
			),
		})
	}

	#[inline]
	#[must_use]
	pub fn saturating_inc(self, dir: Direction) -> Self {
		match dir {
			Direction::Forward => self.saturating_add(1),
			Direction::Backward => self.saturating_sub(1),
		}
	}

	#[inline]
	#[must_use]
	pub fn saturating_add(self, add: u64) -> Self {
		match self {
			Self::Normal(i) => Self::Normal(i.saturating_add(add)),
			Self::Backfilled(i) => Self::Backfilled(i.saturating_add(add as i64)),
		}
	}

	#[inline]
	#[must_use]
	pub fn saturating_sub(self, sub: u64) -> Self {
		match self {
			Self::Normal(i) => Self::Normal(i.saturating_sub(sub)),
			Self::Backfilled(i) => Self::Backfilled(i.saturating_sub(sub as i64)),
		}
	}

	#[inline]
	#[must_use]
	pub fn min() -> Self { Self::Backfilled(i64::MIN) }

	#[inline]
	#[must_use]
	pub fn max() -> Self { Self::Normal(i64::MAX as u64) }

	#[inline]
	pub(crate) fn debug_assert_valid(&self) {
		if let Self::Backfilled(i) = self {
			debug_assert!(*i <= 0, "Backfilled sequence must be negative");
		}
	}
}

impl Display for PduCount {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
		self.debug_assert_valid();
		match self {
			Self::Normal(i) => write!(f, "{i}"),
			Self::Backfilled(i) => write!(f, "{i}"),
		}
	}
}

impl FromStr for PduCount {
	type Err = Error;

	fn from_str(token: &str) -> Result<Self, Self::Err> { Ok(Self::from_signed(token.parse()?)) }
}

impl PartialOrd for PduCount {
	fn partial_cmp(&self, other: &Self) -> Option<Ordering> { Some(self.cmp(other)) }
}

impl Ord for PduCount {
	fn cmp(&self, other: &Self) -> Ordering { self.into_signed().cmp(&other.into_signed()) }
}

impl Default for PduCount {
	fn default() -> Self { Self::Normal(0) }
}
