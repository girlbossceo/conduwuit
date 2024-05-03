# Generic deployment documentation

### Please note that this documentation is not fully representative of conduwuit at the moment. Assume majority of it is outdated.

> ## Getting help
>
> If you run into any problems while setting up conduwuit, ask us
> in `#conduwuit:puppygock.gay` or [open an issue on GitHub](https://github.com/girlbossceo/conduwuit/issues/new).

## Installing conduwuit

You may simply download the binary that fits your machine. Run `uname -m` to see what you need.

Prebuilt binaries can be downloaded from the latest tagged release [here](https://github.com/girlbossceo/conduwuit/releases/latest).

Alternatively, you may compile the binary yourself. First, install any dependencies:

```bash
# Debian
$ sudo apt install libclang-dev build-essential

# RHEL
$ sudo dnf install clang
```
Then, `cd` into the source tree of conduwuit and run:
```bash
$ cargo build --release
```

## Adding a conduwuit user

While conduwuit can run as any user it is usually better to use dedicated users for different services. This also allows
you to make sure that the file permissions are correctly set up.

In Debian or RHEL, you can use this command to create a conduwuit user:

```bash
sudo adduser --system conduwuit --group --disabled-login --no-create-home
```

## Forwarding ports in the firewall or the router

conduwuit uses the ports 443 and 8448 both of which need to be open in the firewall.

If conduwuit runs behind a router or in a container and has a different public IP address than the host system these public ports need to be forwarded directly or indirectly to the port mentioned in the config.

## Setting up a systemd service

Now we'll set up a systemd service for conduwuit, so it's easy to start/stop conduwuit and set it to autostart when your
server reboots. Simply paste the default systemd service you can find below into
`/etc/systemd/system/conduwuit.service`.

```systemd
[Unit]
Description=conduwuit Matrix Server
After=network.target

[Service]
Environment="CONDUWUIT_CONFIG=/etc/conduwuit/conduwuit.toml"
User=conduwuit
Group=conduwuit
RuntimeDirectory=conduwuit
RuntimeDirectoryMode=0750
Restart=always
ExecStart=/usr/local/bin/conduwuit

[Install]
WantedBy=multi-user.target
```

Finally, run

```bash
$ sudo systemctl daemon-reload
```

## Creating the conduwuit configuration file

Now we need to create the conduwuit's config file in `/etc/conduwuit/conduwuit.toml`. Paste this in **and take a moment
to read it. You need to change at least the server name.**
RocksDB (`rocksdb`) is the only supported database backend. SQLite only exists for historical reasons and is not recommended. Any performance issues, storage issues, database issues, etc will not be assisted if using SQLite and you will be asked to migrate to RocksDB first.

See the following example config at [conduwuit-example.toml](../configuration.md)

## Setting the correct file permissions

As we are using a conduwuit specific user we need to allow it to read the config. To do that you can run this command on
Debian or RHEL:

```bash
sudo chown -R root:root /etc/conduwuit
sudo chmod 755 /etc/conduwuit
```

If you use the default database path you also need to run this:

```bash
sudo mkdir -p /var/lib/conduwuit/
sudo chown -R conduwuit:conduwuit /var/lib/conduwuit/
sudo chmod 700 /var/lib/conduwuit/
```

## Setting up the Reverse Proxy

Refer to the documentation or various guides online of your chosen reverse proxy software. A Caddy example will be provided as this is the recommended reverse proxy for new users and is very trivial.

### Caddy

Create `/etc/caddy/conf.d/conduwuit_caddyfile` and enter this (substitute for your server name).

```caddy
your.server.name, your.server.name:8448 {
        # TCP
        reverse_proxy 127.0.0.1:6167

        # UNIX socket
        #reverse_proxy unix//run/conduwuit/conduwuit.sock
}
```

That's it! Just start or enable the service and you're set.

```bash
$ sudo systemctl enable caddy
```

## You're done!

Now you can start conduwuit with:

```bash
$ sudo systemctl start conduwuit
```

Set it to start automatically when your system boots with:

```bash
$ sudo systemctl enable conduwuit
```

## How do I know it works?

You can open [a Matrix client](https://matrix.org/ecosystem/clients), enter your homeserver and try to register.

You can also use these commands as a quick health check.

```bash
$ curl https://your.server.name/_conduwuit/server_version

# If using port 8448
$ curl https://your.server.name:8448/_conduwuit/server_version
```

- To check if your server can talk with other homeservers, you can use the [Matrix Federation Tester](https://federationtester.matrix.org/).
  If you can register but cannot join federated rooms check your config again and also check if the port 8448 is open and forwarded correctly.

# What's next?

## Audio/Video calls

For Audio/Video call functionality see the [TURN Guide](../turn.md).

## Appservices

If you want to set up an appservice, take a look at the [Appservice Guide](../appservices.md).
