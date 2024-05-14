# conduwuit for Debian

Information about downloading and deploying the Debian package. This may also be referenced for other `apt`-based distros such as Ubuntu.

### Installation

It is recommended to see the [generic deployment guide](../deploying/generic.md) for further information if needed as usage of the Debian package is generally related.

### Configuration

When installed, the example config is placed at `/etc/conduwuit/conduwuit.toml` as the default config. At the minimum, you will need to change your `server_name` here.

You can tweak more detailed settings by uncommenting and setting the config options
in `/etc/conduwuit/conduwuit.toml`.

### Running

The package uses the [`conduwuit.service`](../configuration.md#example-systemd-unit-file) systemd unit file to start and stop conduwuit. The binary is installed at `/usr/sbin/conduwuit`.

This package assumes by default that conduwuit will be placed behind a reverse proxy. The default config options apply (listening on `localhost` and TCP port `6167`). Matrix federation requires a valid domain name and TLS, so you will need to set up TLS certificates and renewal for it to work properly if you intend to federate.

Consult various online documentation and guides on setting up a reverse proxy and TLS. Caddy is documented at the [generic deployment guide](../deploying/generic.md#setting-up-the-reverse-proxy) as it's the easiest and most user friendly.
