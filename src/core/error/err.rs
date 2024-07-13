#[macro_export]
macro_rules! Err {
	($($args:tt)*) => {
		Err($crate::err!($($args)*))
	};
}

#[macro_export]
macro_rules! err {
	(error!($($args:tt),+)) => {{
		$crate::error!($($args),+);
		$crate::error::Error::Err(std::format!($($args),+))
	}};

	(debug_error!($($args:tt),+)) => {{
		$crate::debug_error!($($args),+);
		$crate::error::Error::Err(std::format!($($args),+))
	}};

	($variant:ident(error!($($args:tt),+))) => {{
		$crate::error!($($args),+);
		$crate::error::Error::$variant(std::format!($($args),+))
	}};

	($variant:ident(debug_error!($($args:tt),+))) => {{
		$crate::debug_error!($($args),+);
		$crate::error::Error::$variant(std::format!($($args),+))
	}};

	(Config($item:literal, $($args:tt),+)) => {{
		$crate::error!(config = %$item, $($args),+);
		$crate::error::Error::Config($item, std::format!($($args),+))
	}};

	($variant:ident($($args:tt),+)) => {
		$crate::error::Error::$variant(std::format!($($args),+))
	};

	($string:literal$(,)? $($args:tt),*) => {
		$crate::error::Error::Err(std::format!($string, $($args),*))
	};
}
