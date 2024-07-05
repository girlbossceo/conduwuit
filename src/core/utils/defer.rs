#[macro_export]
macro_rules! defer {
	($body:block) => {
		struct _Defer_<F: FnMut()> {
			closure: F,
		}

		impl<F: FnMut()> Drop for _Defer_<F> {
			fn drop(&mut self) { (self.closure)(); }
		}

		let _defer_ = _Defer_ {
			closure: || $body,
		};
	};
}
