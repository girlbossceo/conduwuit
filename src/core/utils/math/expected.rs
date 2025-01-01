use num_traits::ops::checked::{CheckedAdd, CheckedDiv, CheckedMul, CheckedRem, CheckedSub};

use crate::expected;

pub trait Expected {
	#[inline]
	#[must_use]
	fn expected_add(self, rhs: Self) -> Self
	where
		Self: CheckedAdd + Sized,
	{
		expected!(self + rhs)
	}

	#[inline]
	#[must_use]
	fn expected_sub(self, rhs: Self) -> Self
	where
		Self: CheckedSub + Sized,
	{
		expected!(self - rhs)
	}

	#[inline]
	#[must_use]
	fn expected_mul(self, rhs: Self) -> Self
	where
		Self: CheckedMul + Sized,
	{
		expected!(self * rhs)
	}

	#[inline]
	#[must_use]
	fn expected_div(self, rhs: Self) -> Self
	where
		Self: CheckedDiv + Sized,
	{
		expected!(self / rhs)
	}

	#[inline]
	#[must_use]
	fn expected_rem(self, rhs: Self) -> Self
	where
		Self: CheckedRem + Sized,
	{
		expected!(self % rhs)
	}
}

impl<T> Expected for T {}
