//! Integration with `clap`

use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// Commandline arguments
#[derive(Parser, Debug)]
#[clap(version, about, long_about = None)]
pub struct Args {
	#[arg(short, long)]
	/// Optional argument to the path of a conduwuit config TOML file
	pub config: Option<PathBuf>,

	#[clap(subcommand)]
	/// Optional subcommand to export the homeserver signing key and exit
	pub signing_key: Option<SigningKey>,
}

#[derive(Debug, Subcommand)]
pub enum SigningKey {
	/// Filesystem path to export the homeserver signing key to.
	/// The output will be: `ed25519 <version> <keypair base64 encoded>` which
	/// is Synapse's format
	ExportPath {
		path: PathBuf,
	},

	/// Filesystem path for conduwuit to attempt to read and import the
	/// homeserver signing key. The expected format is Synapse's format:
	/// `ed25519 <version> <keypair base64 encoded>`
	ImportPath {
		path: PathBuf,

		#[arg(long)]
		/// Optional argument to import the key but don't overwrite our signing
		/// key, and instead add it to `old_verify_keys`. This field tells other
		/// servers that this is our old public key that can still be used to
		/// sign old events.
		///
		/// See https://spec.matrix.org/v1.9/server-server-api/#get_matrixkeyv2server for more details.
		add_to_old_public_keys: bool,

		#[arg(long)]
		/// Timestamp (`expired_ts`) in seconds since UNIX epoch that the old
		/// homeserver signing key stopped being used.
		///
		/// See https://spec.matrix.org/v1.9/server-server-api/#get_matrixkeyv2server for more details.
		timestamp: u64,
	},
}

/// Parse commandline arguments into structured data
pub fn parse() -> Args { Args::parse() }
