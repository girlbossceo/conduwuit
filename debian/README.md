# conduwuit for Debian

Installation
------------

Information about downloading, building and deploying the Debian package, see
the "Installing conduwuit" section in the Deploying docs.
All following sections until "Setting up the Reverse Proxy" be ignored because
this is handled automatically by the packaging.

Configuration
-------------

When installed, Debconf generates the configuration of the homeserver
(host)name, the address and port it listens on. This configuration ends up in
`/etc/conduwuit/conduwuit.toml`.

You can tweak more detailed settings by uncommenting and setting the variables
in `/etc/conduwuit/conduwuit.toml`. This involves settings such as the maximum
file size for download/upload, enabling federation, etc.

Running
-------

The package uses the `conduwuit.service` systemd unit file to start and
stop conduwuit. It loads the configuration file mentioned above to set up the
environment before running the server.

This package assumes by default that conduwuit will be placed behind a reverse
proxy. This default deployment entails just listening
on `127.0.0.1` and the free port `6167` and is reachable via a client using the URL
<http://localhost:6167>. Matrix federation requires TLS, so you will need to set up
some certificates and renewal, for it to work properly.
