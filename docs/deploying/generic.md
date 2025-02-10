# Generic deployment documentation

> ### Getting help
>
> If you run into any problems while setting up conduwuit, ask us in
> `#conduwuit:puppygock.gay` or [open an issue on
> GitHub](https://github.com/girlbossceo/conduwuit/issues/new).

## Installing conduwuit

### Static prebuilt binary

You may simply download the binary that fits your machine architecture (x86_64
or aarch64). Run `uname -m` to see what you need.

Prebuilt fully static musl binaries can be downloaded from the latest tagged
release [here](https://github.com/girlbossceo/conduwuit/releases/latest) or
`main` CI branch workflow artifact output. These also include Debian/Ubuntu
packages.

Binaries are also available on my website directly at: <https://pup.systems/~strawberry/conduwuit/>

These can be curl'd directly from. `ci-bins` are CI workflow binaries by commit
hash/revision, and `releases` are tagged releases. Sort by descending last
modified for the latest.

These binaries have jemalloc and io_uring statically linked and included with
them, so no additional dynamic dependencies need to be installed.

For the **best** performance; if using an `x86_64` CPU made in the last ~15 years,
we recommend using the `-haswell-` optimised binaries. This sets
`-march=haswell` which is the most compatible and highest performance with
optimised binaries. The database backend, RocksDB, most benefits from this as it
will then use hardware accelerated CRC32 hashing/checksumming which is critical
for performance.

### Compiling

Alternatively, you may compile the binary yourself. We recommend using
Nix (or [Lix](https://lix.systems)) to build conduwuit as this has the most
guaranteed reproducibiltiy and easiest to get a build environment and output
going. This also allows easy cross-compilation.

You can run the `nix build -L .#static-x86_64-linux-musl-all-features` or
`nix build -L .#static-aarch64-linux-musl-all-features` commands based
on architecture to cross-compile the necessary static binary located at
`result/bin/conduwuit`. This is reproducible with the static binaries produced
in our CI.

If wanting to build using standard Rust toolchains, make sure you install:
- `liburing-dev` on the compiling machine, and `liburing` on the target host
- LLVM and libclang for RocksDB

You can build conduwuit using `cargo build --release --all-features`

## Migrating from Conduit

As mentioned in the README, there is little to no steps needed to migrate
from Conduit. As long as you are using the RocksDB database backend, just
replace the binary / container image / etc.

**WARNING**: As of conduwuit 0.5.0, all database and backwards compatibility
with Conduit is no longer supported. We only support migrating *from* Conduit,
not back to Conduit like before. If you are truly finding yourself wanting to
migrate back to Conduit, we would appreciate all your feedback and if we can
assist with any issues or concerns.

**Note**: If you are relying on Conduit's "automatic delegation" feature,
this will **NOT** work on conduwuit and you must configure delegation manually.
This is not a mistake and no support for this feature will be added.

If you are using SQLite, you **MUST** migrate to RocksDB. You can use this
tool to migrate from SQLite to RocksDB: <https://github.com/ShadowJonathan/conduit_toolbox/>

See the `[global.well_known]` config section, or configure your web server
appropriately to send the delegation responses.

## Adding a conduwuit user

While conduwuit can run as any user it is better to use dedicated users for
different services. This also allows you to make sure that the file permissions
are correctly set up.

In Debian, you can use this command to create a conduwuit user:

```bash
sudo adduser --system conduwuit --group --disabled-login --no-create-home
```

For distros without `adduser` (or where it's a symlink to `useradd`):

```bash
sudo useradd -r --shell /usr/bin/nologin --no-create-home conduwuit
```

## Forwarding ports in the firewall or the router

Matrix's default federation port is port 8448, and clients must be using port 443.
If you would like to use only port 443, or a different port, you will need to setup
delegation. conduwuit has config options for doing delegation, or you can configure
your reverse proxy to manually serve the necessary JSON files to do delegation
(see the `[global.well_known]` config section).

If conduwuit runs behind a router or in a container and has a different public
IP address than the host system these public ports need to be forwarded directly
or indirectly to the port mentioned in the config.

Note for NAT users; if you have trouble connecting to your server from the inside
of your network, you need to research your router and see if it supports "NAT
hairpinning" or "NAT loopback".

If your router does not support this feature, you need to research doing local
DNS overrides and force your Matrix DNS records to use your local IP internally.
This can be done at the host level using `/etc/hosts`. If you need this to be
on the network level, consider something like NextDNS or Pi-Hole.

## Setting up a systemd service

Two example systemd units for conduwuit can be found
[on the configuration page](../configuration/examples.md#debian-systemd-unit-file).
You may need to change the `ExecStart=` path to where you placed the conduwuit
binary if it is not `/usr/bin/conduwuit`.

On systems where rsyslog is used alongside journald (i.e. Red Hat-based distros
and OpenSUSE), put `$EscapeControlCharactersOnReceive off` inside
`/etc/rsyslog.conf` to allow color in logs.

If you are using a different `database_path` other than the systemd unit
configured default `/var/lib/conduwuit`, you need to add your path to the
systemd unit's `ReadWritePaths=`. This can be done by either directly editing
`conduwuit.service` and reloading systemd, or running `systemctl edit conduwuit.service`
and entering the following:

```
[Service]
ReadWritePaths=/path/to/custom/database/path
```

## Creating the conduwuit configuration file

Now we need to create the conduwuit's config file in
`/etc/conduwuit/conduwuit.toml`. The example config can be found at
[conduwuit-example.toml](../configuration/examples.md).

**Please take a moment to read the config. You need to change at least the
server name.**

RocksDB is the only supported database backend.

## Setting the correct file permissions

If you are using a dedicated user for conduwuit, you will need to allow it to
read the config. To do that you can run this:

```bash
sudo chown -R root:root /etc/conduwuit
sudo chmod -R 755 /etc/conduwuit
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
header, making federation non-functional. If a workaround is found, feel free to share to get it added to the documentation here.

If using Apache, you need to use `nocanon` in your `ProxyPass` directive to prevent this (note that Apache isn't very good as a general reverse proxy and we discourage the usage of it if you can).

If using Nginx, you need to give conduwuit the request URI using `$request_uri`, or like so:
- `proxy_pass http://127.0.0.1:6167$request_uri;`
- `proxy_pass http://127.0.0.1:6167;`

Nginx users need to increase `client_max_body_size` (default is 1M) to match
`max_request_size` defined in conduwuit.toml.

You will need to reverse proxy everything under following routes:
- `/_matrix/` - core Matrix C-S and S-S APIs
- `/_conduwuit/` - ad-hoc conduwuit routes such as `/local_user_count` and
`/server_version`

You can optionally reverse proxy the following individual routes:
- `/.well-known/matrix/client` and `/.well-known/matrix/server` if using
conduwuit to perform delegation (see the `[global.well_known]` config section)
- `/.well-known/matrix/support` if using conduwuit to send the homeserver admin
contact and support page (formerly known as MSC1929)
- `/` if you would like to see `hewwo from conduwuit woof!` at the root

See the following spec pages for more details on these files:
- [`/.well-known/matrix/server`](https://spec.matrix.org/latest/client-server-api/#getwell-knownmatrixserver)
- [`/.well-known/matrix/client`](https://spec.matrix.org/latest/client-server-api/#getwell-knownmatrixclient)
- [`/.well-known/matrix/support`](https://spec.matrix.org/latest/client-server-api/#getwell-knownmatrixsupport)

Examples of delegation:
- <https://puppygock.gay/.well-known/matrix/server>
- <https://puppygock.gay/.well-known/matrix/client>

### Caddy

Create `/etc/caddy/conf.d/conduwuit_caddyfile` and enter this (substitute for
your server name).

```caddyfile
your.server.name, your.server.name:8448 {
    # TCP reverse_proxy
    reverse_proxy 127.0.0.1:6167
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
