/// one true function for returning the conduwuit version with the necessary
/// CONDUWUIT_VERSION_EXTRA env variables used if specified
///
/// Set the environment variable `CONDUWUIT_VERSION_EXTRA` to any UTF-8 string
/// to include it in parenthesis after the SemVer version. A common value are
/// git commit hashes.
#[must_use]
pub fn conduwuit() -> String {
	match option_env!("CONDUWUIT_VERSION_EXTRA") {
		Some(extra) => {
			if extra.is_empty() {
				env!("CARGO_PKG_VERSION").to_owned()
			} else {
				format!("{} ({})", env!("CARGO_PKG_VERSION"), extra)
			}
		},
		None => match option_env!("CONDUIT_VERSION_EXTRA") {
			Some(extra) => {
				if extra.is_empty() {
					env!("CARGO_PKG_VERSION").to_owned()
				} else {
					format!("{} ({})", env!("CARGO_PKG_VERSION"), extra)
				}
			},
			None => env!("CARGO_PKG_VERSION").to_owned(),
		},
	}
}
