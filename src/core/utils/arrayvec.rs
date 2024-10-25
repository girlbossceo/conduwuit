use ::arrayvec::ArrayVec;

pub trait ArrayVecExt<T> {
	fn extend_from_slice(&mut self, other: &[T]) -> &mut Self;
}

impl<T: Copy, const CAP: usize> ArrayVecExt<T> for ArrayVec<T, CAP> {
	#[inline]
	fn extend_from_slice(&mut self, other: &[T]) -> &mut Self {
		self.try_extend_from_slice(other)
			.expect("Insufficient buffer capacity to extend from slice");

		self
	}
}
