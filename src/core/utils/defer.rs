#[macro_export]
macro_rules! defer {
	($body:block) => {
		struct _Defer_<F>
		where
			F: FnMut(),
		{
			closure: F,
		}

		impl<F> Drop for _Defer_<F>
		where
			F: FnMut(),
		{
			fn drop(&mut self) { (self.closure)(); }
		}

		let _defer_ = _Defer_ {
			closure: || $body,
		};
	};
}
