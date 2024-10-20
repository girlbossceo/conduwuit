# Generic deployment documentation

> ## Getting help
>
> If you run into any problems while setting up conduwuit, ask us in
> `#conduwuit:puppygock.gay` or [open an issue on
> GitHub](https://github.com/girlbossceo/conduwuit/issues/new).

## Installing conduwuit

You may simply download the binary that fits your machine. Run `uname -m` to see
what you need.

Prebuilt fully static musl binaries can be downloaded from the latest tagged
release [here](https://github.com/girlbossceo/conduwuit/releases/latest) or
`main` CI branch workflow artifact output. These also include Debian/Ubuntu packages.

These binaries have jemalloc and io_uring statically linked and included with
them, so no additional dynamic dependencies need to be installed.

Alternatively, you may compile the binary yourself. We recommend using
Nix (or [Lix](https://lix.systems)) to build conduwuit as this has the most guaranteed
reproducibiltiy and easiest to get a build environment and output going. This also
allows easy cross-compilation.

You can run the `nix build -L .#static-x86_64-unknown-linux-musl-all-features` or
`nix build -L .#static-aarch64-unknown-linux-musl-all-features` commands based
on architecture to cross-compile the necessary static binary located at
`result/bin/conduit`. This is reproducible with the static binaries produced in our CI.

Otherwise, follow standard Rust project build guides (installing git and cloning
the repo, getting the Rust toolchain via rustup, installing LLVM toolchain +
libclang for RocksDB, installing liburing for io_uring and RocksDB, etc).

## Migrating from Conduit

As mentioned in the README, there is little to no steps needed to migrate
from Conduit. As long as you are using the RocksDB database backend, just
replace the binary / container image / etc.

**Note**: If you are relying on Conduit's "automatic delegation" feature,
this will **NOT** work on conduwuit and you must configure delegation manually.
This is not a mistake and no support for this feature will be added.

See the `[global.well_known]` config section, or configure your web server
appropriately to send the delegation responses.

## Adding a conduwuit user

While conduwuit can run as any user it is better to use dedicated users for
different services. This also allows you to make sure that the file permissions
are correctly set up.

In Debian or Fedora/RHEL, you can use this command to create a conduwuit user:

```bash
sudo adduser --system conduwuit --group --disabled-login --no-create-home
```

For distros without `adduser`:

```bash
sudo useradd -r --shell /usr/bin/nologin --no-create-home conduwuit
```

## Forwarding ports in the firewall or the router

conduwuit uses the ports 443 and 8448 both of which need to be open in the
firewall.

If conduwuit runs behind a router or in a container and has a different public
IP address than the host system these public ports need to be forwarded directly
or indirectly to the port mentioned in the config.

## Setting up a systemd service

The systemd unit for conduwuit can be found
[here](../configuration/examples.md#example-systemd-unit-file). You may need to
change the `ExecStart=` path to where you placed the conduwuit binary.

On systems where rsyslog is used alongside journald (i.e. Red Hat-based distros and OpenSUSE), put `$EscapeControlCharactersOnReceive off` inside `/etc/rsyslog.conf` to allow color in logs.

## Creating the conduwuit configuration file

Now we need to create the conduwuit's config file in
`/etc/conduwuit/conduwuit.toml`. The example config can be found at
[conduwuit-example.toml](../configuration/examples.md).

**Please take a moment to read the config. You need to change at least the server name.**

RocksDB is the only supported database backend.

## Setting the correct file permissions

If you are using a dedicated user for conduwuit, you will need to allow it to
read the config. To do that you can run this:

```bash
sudo chown -R root:root /etc/conduwuit sudo chmod -R 755 /etc/conduwuit
```

If you use the default database path you also need to run this:

```bash
sudo mkdir -p /var/lib/conduwuit/
sudo chown -R conduwuit:conduwuit /var/lib/conduwuit/
sudo chmod 700 /var/lib/conduwuit/
```

## Setting up the Reverse Proxy

Refer to the documentation or various guides online of your chosen reverse proxy
software. There are many examples of basic Apache/Nginx reverse proxy setups
out there.

A [Caddy](https://caddyserver.com/) example will be provided as this
is the recommended reverse proxy for new users and is very trivial to use
(handles TLS, reverse proxy headers, etc transparently with proper defaults).

Lighttpd is not supported as it seems to mess with the `X-Matrix` Authorization
header, making federation non-functional. If using Apache, you need to use
`nocanon` in your `ProxyPass` directive to prevent this (note that Apache
isn't very good as a general reverse proxy).

Nginx users may need to set `proxy_buffering off;` if there are issues with
uploading media like images.

You will need to reverse proxy everything under following routes:
- `/_matrix/` - core Matrix C-S and S-S APIs
- `/_conduwuit/` - ad-hoc conduwuit routes such as `/local_user_count` and
`/server_version`

You can optionally reverse proxy the following individual routes:
- `/.well-known/matrix/client` and `/.well-known/matrix/server` if using
conduwuit to perform delegation
- `/.well-known/matrix/support` if using conduwuit to send the homeserver admin
contact and support page (formerly known as MSC1929)
- `/` if you would like to see `hewwo from conduwuit woof!` at the root

### Caddy

Create `/etc/caddy/conf.d/conduwuit_caddyfile` and enter this (substitute for
your server name).

```caddyfile
your.server.name, your.server.name:8448 {
    # TCP reverse_proxy
    127.0.0.1:6167
    # UNIX socket
    #reverse_proxy unix//run/conduwuit/conduwuit.sock
}
```

That's it! Just start and enable the service and you're set.

```bash
sudo systemctl enable --now caddy
```

## You're done

Now you can start conduwuit with:

```bash
sudo systemctl start conduwuit
```

Set it to start automatically when your system boots with:

```bash
sudo systemctl enable conduwuit
```

## How do I know it works?

You can open [a Matrix client](https://matrix.org/ecosystem/clients), enter your
homeserver and try to register.

You can also use these commands as a quick health check (replace
`your.server.name`).

```bash
curl https://your.server.name/_conduwuit/server_version

# If using port 8448
curl https://your.server.name:8448/_conduwuit/server_version

# If federation is enabled
curl https://your.server.name:8448/_matrix/federation/v1/version
```

- To check if your server can talk with other homeservers, you can use the
[Matrix Federation Tester](https://federationtester.matrix.org/). If you can
register but cannot join federated rooms check your config again and also check
if the port 8448 is open and forwarded correctly.

# What's next?

## Audio/Video calls

For Audio/Video call functionality see the [TURN Guide](../turn.md).

## Appservices

If you want to set up an appservice, take a look at the [Appservice
Guide](../appservices.md).
