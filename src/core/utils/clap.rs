//! Integration with `clap`

use std::path::PathBuf;

pub use clap::Parser;

use super::conduwuit_version;

/// Commandline arguments
#[derive(Parser, Debug)]
#[clap(version = conduwuit_version(), about, long_about = None)]
pub struct Args {
	#[arg(short, long)]
	/// Optional argument to the path of a conduwuit config TOML file
	pub config: Option<PathBuf>,
}

/// Parse commandline arguments into structured data
#[must_use]
pub fn parse() -> Args { Args::parse() }
