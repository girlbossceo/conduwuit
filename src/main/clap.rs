//! Integration with `clap`

use std::path::PathBuf;

use clap::Parser;
use conduit::{Config, Result};

/// Commandline arguments
#[derive(Parser, Debug)]
#[clap(version = conduit::version(), about, long_about = None)]
pub(crate) struct Args {
	#[arg(short, long)]
	/// Optional argument to the path of a conduwuit config TOML file
	pub(crate) config: Option<PathBuf>,
}

/// Parse commandline arguments into structured data
#[must_use]
pub(super) fn parse() -> Args { Args::parse() }

/// Synthesize any command line options with configuration file options.
pub(crate) fn update(config: &mut Config, args: &Args) -> Result<()> {
	// Indicate the admin console should be spawned automatically if the
	// configuration file hasn't already.
	config.admin_console_automatic |= args.console.unwrap_or(false);

	Ok(())
}
