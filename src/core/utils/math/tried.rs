use num_traits::ops::checked::{CheckedAdd, CheckedDiv, CheckedMul, CheckedRem, CheckedSub};

use crate::{Result, checked};

pub trait Tried {
	#[inline]
	fn try_add(self, rhs: Self) -> Result<Self>
	where
		Self: CheckedAdd + Sized,
	{
		checked!(self + rhs)
	}

	#[inline]
	fn try_sub(self, rhs: Self) -> Result<Self>
	where
		Self: CheckedSub + Sized,
	{
		checked!(self - rhs)
	}

	#[inline]
	fn try_mul(self, rhs: Self) -> Result<Self>
	where
		Self: CheckedMul + Sized,
	{
		checked!(self * rhs)
	}

	#[inline]
	fn try_div(self, rhs: Self) -> Result<Self>
	where
		Self: CheckedDiv + Sized,
	{
		checked!(self / rhs)
	}

	#[inline]
	fn try_rem(self, rhs: Self) -> Result<Self>
	where
		Self: CheckedRem + Sized,
	{
		checked!(self % rhs)
	}
}

impl<T> Tried for T {}
