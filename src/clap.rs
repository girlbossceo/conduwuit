//! Integration with `clap`

use std::path::PathBuf;

use clap::Parser;

/// Returns the current version of the crate with extra info if supplied
///
/// Set the environment variable `CONDUIT_VERSION_EXTRA` to any UTF-8 string to
/// include it in parenthesis after the SemVer version. A common value are git
/// commit hashes.
fn version() -> String {
	let cargo_pkg_version = env!("CARGO_PKG_VERSION");

	match option_env!("CONDUIT_VERSION_EXTRA") {
		Some(x) => format!("{} ({})", cargo_pkg_version, x),
		None => cargo_pkg_version.to_owned(),
	}
}

/// Commandline arguments
#[derive(Parser, Debug)]
#[clap(version = version(), about, long_about = None)]
pub struct Args {
	#[arg(short, long)]
	/// Optional argument to the path of a conduwuit config TOML file
	pub config: Option<PathBuf>,
}

/// Parse commandline arguments into structured data
pub fn parse() -> Args { Args::parse() }
