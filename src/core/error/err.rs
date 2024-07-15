//! Error construction macros
//!
//! These are specialized macros specific to this project's patterns for
//! throwing Errors; they make Error construction succinct and reduce clutter.
//! They are developed from folding existing patterns into the macro while
//! fixing several anti-patterns in the codebase.
//!
//! - The primary macros `Err!` and `err!` are provided. `Err!` simply wraps
//!   `err!` in the Result variant to reduce `Err(err!(...))` boilerplate, thus
//!   `err!` can be used in any case.
//!
//! 1. The macro makes the general Error construction easy: `return
//!    Err!("something went wrong")` replaces the prior `return
//!    Err(Error::Err("something went wrong".to_owned()))`.
//!
//! 2. The macro integrates format strings automatically: `return
//!    Err!("something bad: {msg}")` replaces the prior `return
//!    Err(Error::Err(format!("something bad: {msg}")))`.
//!
//! 3. The macro scopes variants of Error: `return Err!(Database("problem with
//!    bad database."))` replaces the prior `return Err(Error::Database("problem
//!    with bad database."))`.
//!
//! 4. The macro matches and scopes some special-case sub-variants, for example
//!    with ruma ErrorKind: `return Err!(Request(MissingToken("you must provide
//!    an access token")))`.
//!
//! 5. The macro fixes the anti-pattern of repeating messages in an error! log
//!    and then again in an Error construction, often slightly different due to
//!    the Error variant not supporting a format string. Instead `return
//!    Err(Database(error!("problem with db: {msg}")))` logs the error at the
//!    callsite and then returns the error with the same string. Caller has the
//!    option of replacing `error!` with `debug_error!`.
#[macro_export]
macro_rules! Err {
	($($args:tt)*) => {
		Err($crate::err!($($args)*))
	};
}

#[macro_export]
macro_rules! err {
	(Config($item:literal, $($args:expr),*)) => {{
		$crate::error!(config = %$item, $($args),*);
		$crate::error::Error::Config($item, $crate::format_maybe!($($args),*))
	}};

	(Request(Forbidden($level:ident!($($args:expr),*)))) => {{
		$crate::$level!($($args),*);
		$crate::error::Error::Request(
			::ruma::api::client::error::ErrorKind::forbidden(),
			$crate::format_maybe!($($args),*),
			::http::StatusCode::BAD_REQUEST
		)
	}};

	(Request(Forbidden($($args:expr),*))) => {
		$crate::error::Error::Request(
			::ruma::api::client::error::ErrorKind::forbidden(),
			$crate::format_maybe!($($args),*),
			::http::StatusCode::BAD_REQUEST
		)
	};

	(Request($variant:ident($level:ident!($($args:expr),*)))) => {{
		$crate::$level!($($args),*);
		$crate::error::Error::Request(
			::ruma::api::client::error::ErrorKind::$variant,
			$crate::format_maybe!($($args),*),
			::http::StatusCode::BAD_REQUEST
		)
	}};

	(Request($variant:ident($($args:expr),*))) => {
		$crate::error::Error::Request(
			::ruma::api::client::error::ErrorKind::$variant,
			$crate::format_maybe!($($args),*),
			::http::StatusCode::BAD_REQUEST
		)
	};

	($variant:ident($level:ident!($($args:expr),*))) => {{
		$crate::$level!($($args),*);
		$crate::error::Error::$variant($crate::format_maybe!($($args),*))
	}};

	($variant:ident($($args:expr),*)) => {
		$crate::error::Error::$variant($crate::format_maybe!($($args),*))
	};

	($level:ident!($($args:expr),*)) => {{
		$crate::$level!($($args),*);
		$crate::error::Error::Err($crate::format_maybe!($($args),*))
	}};

	($($args:expr),*) => {
		$crate::error::Error::Err($crate::format_maybe!($($args),*))
	};
}
