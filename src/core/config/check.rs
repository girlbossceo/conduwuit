use figment::Figment;

use super::DEPRECATED_KEYS;
use crate::{debug, debug_info, error, info, warn, Config, Err, Result};

#[allow(clippy::cognitive_complexity)]
pub fn check(config: &Config) -> Result<()> {
	if cfg!(debug_assertions) {
		info!("Note: conduwuit was built without optimisations (i.e. debug build)");
	}

	warn_deprecated(config);
	warn_unknown_key(config);

	if config.sentry && config.sentry_endpoint.is_none() {
		return Err!(Config("sentry_endpoint", "Sentry cannot be enabled without an endpoint set"));
	}

	if cfg!(all(feature = "hardened_malloc", feature = "jemalloc")) {
		warn!(
			"hardened_malloc and jemalloc are both enabled, this causes jemalloc to be used. If using --all-features, \
			 this is harmless."
		);
	}

	if cfg!(not(unix)) && config.unix_socket_path.is_some() {
		return Err!(Config(
			"unix_socket_path",
			"UNIX socket support is only available on *nix platforms. Please remove 'unix_socket_path' from your \
			 config."
		));
	}

	if cfg!(unix) && config.unix_socket_path.is_none() {
		config.get_bind_addrs().iter().for_each(|addr| {
			use std::path::Path;

			if addr.ip().is_loopback() {
				debug_info!("Found loopback listening address {addr}, running checks if we're in a container.");

				if Path::new("/proc/vz").exists() /* Guest */ && !Path::new("/proc/bz").exists()
				/* Host */
				{
					error!(
						"You are detected using OpenVZ with a loopback/localhost listening address of {addr}. If you \
						 are using OpenVZ for containers and you use NAT-based networking to communicate with the \
						 host and guest, this will NOT work. Please change this to \"0.0.0.0\". If this is expected, \
						 you can ignore.",
					);
				}

				if Path::new("/.dockerenv").exists() {
					error!(
						"You are detected using Docker with a loopback/localhost listening address of {addr}. If you \
						 are using a reverse proxy on the host and require communication to conduwuit in the Docker \
						 container via NAT-based networking, this will NOT work. Please change this to \"0.0.0.0\". \
						 If this is expected, you can ignore.",
					);
				}

				if Path::new("/run/.containerenv").exists() {
					error!(
						"You are detected using Podman with a loopback/localhost listening address of {addr}. If you \
						 are using a reverse proxy on the host and require communication to conduwuit in the Podman \
						 container via NAT-based networking, this will NOT work. Please change this to \"0.0.0.0\". \
						 If this is expected, you can ignore.",
					);
				}
			}
		});
	}

	// rocksdb does not allow max_log_files to be 0
	if config.rocksdb_max_log_files == 0 {
		return Err!(Config(
			"max_log_files",
			"rocksdb_max_log_files cannot be 0. Please set a value at least 1."
		));
	}

	// yeah, unless the user built a debug build hopefully for local testing only
	if cfg!(not(debug_assertions)) && config.server_name == "your.server.name" {
		return Err!(Config(
			"server_name",
			"You must specify a valid server name for production usage of conduwuit."
		));
	}

	// check if the user specified a registration token as `""`
	if config.registration_token == Some(String::new()) {
		return Err!(Config(
			"registration_token",
			"Registration token was specified but is empty (\"\")"
		));
	}

	// check if we can read the token file path, and check if the file is empty
	if config.registration_token_file.as_ref().is_some_and(|path| {
		let Ok(token) = std::fs::read_to_string(path).inspect_err(|e| {
			error!("Failed to read the registration token file: {e}");
		}) else {
			return true;
		};

		token == String::new()
	}) {
		return Err!(Config(
			"registration_token_file",
			"Registration token file was specified but is empty or failed to be read"
		));
	}

	if config.max_request_size < 5_120_000 {
		return Err!(Config(
			"max_request_size",
			"Max request size is less than 5MB. Please increase it."
		));
	}

	// check if user specified valid IP CIDR ranges on startup
	for cidr in &config.ip_range_denylist {
		if let Err(e) = ipaddress::IPAddress::parse(cidr) {
			return Err!(Config("ip_range_denylist", "Parsing specified IP CIDR range from string: {e}."));
		}
	}

	if config.allow_registration
		&& !config.yes_i_am_very_very_sure_i_want_an_open_registration_server_prone_to_abuse
		&& config.registration_token.is_none()
		&& config.registration_token_file.is_none()
	{
		return Err!(Config(
			"registration_token",
			"!! You have `allow_registration` enabled without a token configured in your config which means you are \
			 allowing ANYONE to register on your conduwuit instance without any 2nd-step (e.g. registration token).\n
If this is not the intended behaviour, please set a registration token.\n
For security and safety reasons, conduwuit will shut down. If you are extra sure this is the desired behaviour you \
			 want, please set the following config option to true:
`yes_i_am_very_very_sure_i_want_an_open_registration_server_prone_to_abuse`"
		));
	}

	if config.allow_registration
		&& config.yes_i_am_very_very_sure_i_want_an_open_registration_server_prone_to_abuse
		&& config.registration_token.is_none()
		&& config.registration_token_file.is_none()
	{
		warn!(
			"Open registration is enabled via setting \
			 `yes_i_am_very_very_sure_i_want_an_open_registration_server_prone_to_abuse` and `allow_registration` to \
			 true without a registration token configured. You are expected to be aware of the risks now.\n
    If this is not the desired behaviour, please set a registration token."
		);
	}

	if config.allow_outgoing_presence && !config.allow_local_presence {
		return Err!(Config(
			"allow_local_presence",
			"Outgoing presence requires allowing local presence. Please enable 'allow_local_presence'."
		));
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

/// Iterates over all the keys in the config file and warns if there is a
/// deprecated key specified
fn warn_deprecated(config: &Config) {
	debug!("Checking for deprecated config keys");
	let mut was_deprecated = false;
	for key in config
		.catchall
		.keys()
		.filter(|key| DEPRECATED_KEYS.iter().any(|s| s == key))
	{
		warn!("Config parameter \"{}\" is deprecated, ignoring.", key);
		was_deprecated = true;
	}

	if was_deprecated {
		warn!(
			"Read conduwuit config documentation at https://conduwuit.puppyirl.gay/configuration.html and check your \
			 configuration if any new configuration parameters should be adjusted"
		);
	}
}

/// iterates over all the catchall keys (unknown config options) and warns
/// if there are any.
fn warn_unknown_key(config: &Config) {
	debug!("Checking for unknown config keys");
	for key in config
		.catchall
		.keys()
		.filter(|key| "config".to_owned().ne(key.to_owned()) /* "config" is expected */)
	{
		warn!("Config parameter \"{}\" is unknown to conduwuit, ignoring.", key);
	}
}

/// Checks the presence of the `address` and `unix_socket_path` keys in the
/// raw_config, exiting the process if both keys were detected.
pub(super) fn is_dual_listening(raw_config: &Figment) -> Result<()> {
	let contains_address = raw_config.contains("address");
	let contains_unix_socket = raw_config.contains("unix_socket_path");
	if contains_address && contains_unix_socket {
		return Err!(
			"TOML keys \"address\" and \"unix_socket_path\" were both defined. Please specify only one option."
		);
	}

	Ok(())
}
