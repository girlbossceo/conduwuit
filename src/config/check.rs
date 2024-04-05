#[cfg(unix)]
use std::path::Path; // not unix specific, just only for UNIX sockets stuff and *nix container checks

use tracing::{debug, error, info, warn};

use crate::{utils::error::Error, Config};

pub fn check(config: &Config) -> Result<(), Error> {
	config.warn_deprecated();
	config.warn_unknown_key();

	if config.unix_socket_path.is_some() && !cfg!(unix) {
		return Err(Error::bad_config(
			"UNIX socket support is only available on *nix platforms. Please remove \"unix_socket_path\" from your \
			 config.",
		));
	}

	if config.address.is_loopback() && cfg!(unix) {
		debug!(
			"Found loopback listening address {}, running checks if we're in a container.",
			config.address
		);

		#[cfg(unix)]
		if Path::new("/proc/vz").exists() /* Guest */ && !Path::new("/proc/bz").exists()
		/* Host */
		{
			error!(
				"You are detected using OpenVZ with a loopback/localhost listening address of {}. If you are using \
				 OpenVZ for containers and you use NAT-based networking to communicate with the host and guest, this \
				 will NOT work. Please change this to \"0.0.0.0\". If this is expected, you can ignore.",
				config.address
			);
		}

		#[cfg(unix)]
		if Path::new("/.dockerenv").exists() {
			error!(
				"You are detected using Docker with a loopback/localhost listening address of {}. If you are using a \
				 reverse proxy on the host and require communication to conduwuit in the Docker container via \
				 NAT-based networking, this will NOT work. Please change this to \"0.0.0.0\". If this is expected, \
				 you can ignore.",
				config.address
			);
		}

		#[cfg(unix)]
		if Path::new("/run/.containerenv").exists() {
			error!(
				"You are detected using Podman with a loopback/localhost listening address of {}. If you are using a \
				 reverse proxy on the host and require communication to conduwuit in the Podman container via \
				 NAT-based networking, this will NOT work. Please change this to \"0.0.0.0\". If this is expected, \
				 you can ignore.",
				config.address
			);
		}
	}

	// rocksdb does not allow max_log_files to be 0
	if config.rocksdb_max_log_files == 0 && cfg!(feature = "rocksdb") {
		return Err(Error::bad_config(
			"When using RocksDB, rocksdb_max_log_files cannot be 0. Please set a value at least 1.",
		));
	}

	// yeah, unless the user built a debug build hopefully for local testing only
	if config.server_name == "your.server.name" && !cfg!(debug_assertions) {
		return Err(Error::bad_config(
			"You must specify a valid server name for production usage of conduwuit.",
		));
	}

	if cfg!(debug_assertions) {
		info!("Note: conduwuit was built without optimisations (i.e. debug build)");
	}

	// check if the user specified a registration token as `""`
	if config.registration_token == Some(String::new()) {
		return Err(Error::bad_config("Registration token was specified but is empty (\"\")"));
	}

	if config.max_request_size < 16384 {
		return Err(Error::bad_config("Max request size is less than 16KB. Please increase it."));
	}

	// check if user specified valid IP CIDR ranges on startup
	for cidr in &config.ip_range_denylist {
		if let Err(e) = ipaddress::IPAddress::parse(cidr) {
			error!("Error parsing specified IP CIDR range from string: {e}");
			return Err(Error::bad_config("Error parsing specified IP CIDR ranges from strings"));
		}
	}

	if config.allow_registration
		&& !config.yes_i_am_very_very_sure_i_want_an_open_registration_server_prone_to_abuse
		&& config.registration_token.is_none()
	{
		return Err(Error::bad_config(
			"!! You have `allow_registration` enabled without a token configured in your config which means you are \
			 allowing ANYONE to register on your conduwuit instance without any 2nd-step (e.g. registration token).\n
If this is not the intended behaviour, please set a registration token with the `registration_token` config option.\n
For security and safety reasons, conduwuit will shut down. If you are extra sure this is the desired behaviour you \
			 want, please set the following config option to true:
`yes_i_am_very_very_sure_i_want_an_open_registration_server_prone_to_abuse`",
		));
	}

	if config.allow_registration
		&& config.yes_i_am_very_very_sure_i_want_an_open_registration_server_prone_to_abuse
		&& config.registration_token.is_none()
	{
		warn!(
			"Open registration is enabled via setting \
			 `yes_i_am_very_very_sure_i_want_an_open_registration_server_prone_to_abuse` and `allow_registration` to \
			 true without a registration token configured. You are expected to be aware of the risks now.\n
    If this is not the desired behaviour, please set a registration token."
		);
	}

	if config.allow_outgoing_presence && !config.allow_local_presence {
		return Err(Error::bad_config(
			"Outgoing presence requires allowing local presence. Please enable \"allow_local_presence\".",
		));
	}

	if config.allow_outgoing_presence {
		warn!(
			"! Outgoing federated presence is not spec compliant due to relying on PDUs and EDUs combined.\nOutgoing \
			 presence will not be very reliable due to this and any issues with federated outgoing presence are very \
			 likely attributed to this issue.\nIncoming presence and local presence are unaffected."
		);
	}

	if config
		.url_preview_domain_contains_allowlist
		.contains(&"*".to_owned())
	{
		warn!(
			"All URLs are allowed for URL previews via setting \"url_preview_domain_contains_allowlist\" to \"*\". \
			 This opens up significant attack surface to your server. You are expected to be aware of the risks by \
			 doing this."
		);
	}
	if config
		.url_preview_domain_explicit_allowlist
		.contains(&"*".to_owned())
	{
		warn!(
			"All URLs are allowed for URL previews via setting \"url_preview_domain_explicit_allowlist\" to \"*\". \
			 This opens up significant attack surface to your server. You are expected to be aware of the risks by \
			 doing this."
		);
	}
	if config
		.url_preview_url_contains_allowlist
		.contains(&"*".to_owned())
	{
		warn!(
			"All URLs are allowed for URL previews via setting \"url_preview_url_contains_allowlist\" to \"*\". This \
			 opens up significant attack surface to your server. You are expected to be aware of the risks by doing \
			 this."
		);
	}

	Ok(())
}
