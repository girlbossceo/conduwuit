//! Integration with `clap`

use std::path::PathBuf;

use clap::Parser;

use super::conduwuit_version;

/// Commandline arguments
#[derive(Parser, Debug)]
#[clap(version = conduwuit_version(), about, long_about = None)]
pub(crate) struct Args {
	#[arg(short, long)]
	/// Optional argument to the path of a conduwuit config TOML file
	pub(crate) config: Option<PathBuf>,
}

/// Parse commandline arguments into structured data
#[must_use]
pub(crate) fn parse() -> Args { Args::parse() }
