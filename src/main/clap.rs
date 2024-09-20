//! Integration with `clap`

use std::path::PathBuf;

use clap::Parser;
use conduit::{
	config::{Figment, FigmentValue},
	err, toml, Err, Result,
};

/// Commandline arguments
#[derive(Parser, Debug)]
#[clap(version = conduit::version(), about, long_about = None)]
pub(crate) struct Args {
	#[arg(short, long)]
	/// Path to the config TOML file (optional)
	pub(crate) config: Option<Vec<PathBuf>>,

	/// Override a configuration variable using TOML 'key=value' syntax
	#[arg(long, short('O'))]
	pub(crate) option: Vec<String>,

	#[cfg(feature = "console")]
	/// Activate admin command console automatically after startup.
	#[arg(long, num_args(0))]
	pub(crate) console: bool,

	/// Execute console command automatically after startup.
	#[arg(long)]
	pub(crate) execute: Vec<String>,

	/// Set functional testing modes if available. Ex '--test=smoke'
	#[arg(long, hide(true))]
	pub(crate) test: Vec<String>,
}

/// Parse commandline arguments into structured data
#[must_use]
pub(super) fn parse() -> Args { Args::parse() }

/// Synthesize any command line options with configuration file options.
pub(crate) fn update(mut config: Figment, args: &Args) -> Result<Figment> {
	#[cfg(feature = "console")]
	// Indicate the admin console should be spawned automatically if the
	// configuration file hasn't already.
	if args.console {
		config = config.join(("admin_console_automatic", true));
	}

	// Execute commands after any commands listed in configuration file
	config = config.adjoin(("admin_execute", &args.execute));

	// Update config with names of any functional-tests
	config = config.adjoin(("test", &args.test));

	// All other individual overrides can go last in case we have options which
	// set multiple conf items at once and the user still needs granular overrides.
	for option in &args.option {
		let (key, val) = option
			.split_once('=')
			.ok_or_else(|| err!("Missing '=' in -O/--option: {option:?}"))?;

		if key.is_empty() {
			return Err!("Missing key= in -O/--option: {option:?}");
		}

		if val.is_empty() {
			return Err!("Missing =val in -O/--option: {option:?}");
		}

		// The value has to pass for what would appear as a line in the TOML file.
		let val = toml::from_str::<FigmentValue>(option)?;
		let FigmentValue::Dict(_, val) = val else {
			panic!("Unexpected Figment Value: {val:#?}");
		};

		// Figment::merge() overrides existing
		config = config.merge((key, val[key].clone()));
	}

	Ok(config)
}
