use crate::{error, error::Error, info, warn, Config, Err};

pub fn check(config: &Config) -> Result<(), Error> {
	#[cfg(debug_assertions)]
	info!("Note: conduwuit was built without optimisations (i.e. debug build)");

	#[cfg(all(feature = "rocksdb", not(feature = "sha256_media")))] // prevents catching this in `--all-features`
	warn!(
		"Note the rocksdb feature was deleted from conduwuit. SQLite support was removed and RocksDB is the only \
		 supported backend now. Please update your build script to remove this feature."
	);
	#[cfg(all(feature = "sha256_media", not(feature = "rocksdb")))] // prevents catching this in `--all-features`
	warn!(
		"Note the sha256_media feature was deleted from conduwuit, it is now fully integrated in a \
		 forwards-compatible way. Please update your build script to remove this feature."
	);

	config.warn_deprecated();
	config.warn_unknown_key();

	if config.sentry && config.sentry_endpoint.is_none() {
		return Err!(Config("sentry_endpoint", "Sentry cannot be enabled without an endpoint set"));
	}

	#[cfg(all(feature = "hardened_malloc", feature = "jemalloc"))]
	warn!(
		"hardened_malloc and jemalloc are both enabled, this causes jemalloc to be used. If using --all-features, \
		 this is harmless."
	);

	#[cfg(not(unix))]
	if config.unix_socket_path.is_some() {
		return Err!(Config(
			"unix_socket_path",
			"UNIX socket support is only available on *nix platforms. Please remove \"unix_socket_path\" from your \
			 config.",
		));
	}

	#[cfg(unix)]
	if config.unix_socket_path.is_none() {
		config.get_bind_addrs().iter().for_each(|addr| {
			use std::path::Path;

			if addr.ip().is_loopback() {
				crate::debug_info!("Found loopback listening address {addr}, running checks if we're in a container.",);

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
	#[cfg(not(debug_assertions))]
	if config.server_name == "your.server.name" {
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
	{
		return Err!(Config(
			"registration_token",
			"!! You have `allow_registration` enabled without a token configured in your config which means you are \
			 allowing ANYONE to register on your conduwuit instance without any 2nd-step (e.g. registration token).\n
If this is not the intended behaviour, please set a registration token with the `registration_token` config option.\n
For security and safety reasons, conduwuit will shut down. If you are extra sure this is the desired behaviour you \
			 want, please set the following config option to true:
`yes_i_am_very_very_sure_i_want_an_open_registration_server_prone_to_abuse`"
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
