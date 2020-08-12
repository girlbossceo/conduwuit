# Deploy from source

## Prerequisites

Make sure you have `libssl-dev` and `pkg-config` installed and the [rust toolchain](https://rustup.rs) is available on at least on user.


## Install Conduit

```bash
$ sudo useradd -m conduit
$ sudo -u conduit cargo install --git "https://git.koesters.xyz/timo/conduit.git"
```


## Setup systemd service

In this guide, we set up a systemd service for Conduit, so it's easy to start, stop Conduit and set it to autostart when your server reboots. Paste the default systemd service below and configure it to fit your setup (in /etc/systemd/system/conduit.service).

```systemd
[Unit]
Description=Conduit
After=network.target

[Service]
Environment="ROCKET_SERVER_NAME=conduit.rs" # EDIT THIS

Environment="ROCKET_PORT=14004" # Reverse proxy port

#Environment="ROCKET_REGISTRATION_DISABLED=true"
#Environment="ROCKET_LOG=normal" # Detailed logging

Environment="ROCKET_ENV=production"
User=conduit
Group=conduit
Type=simple
Restart=always
ExecStart=/home/conduit/.cargo/bin/conduit

[Install]
WantedBy=multi-user.target
```

Finally, run
```bash
$ sudo systemctl daemon-reload
```


## Setup Reverse Proxy

This depends on whether you use Apache, Nginx or something else. For Apache it looks like this (in /etc/apache2/sites-enabled/050-conduit.conf):
```
<VirtualHost *:443>

ServerName conduit.koesters.xyz # EDIT THIS

AllowEncodedSlashes NoDecode

ServerAlias conduit.koesters.xyz # EDIT THIS

ProxyPreserveHost On
ProxyRequests off
AllowEncodedSlashes NoDecode
ProxyPass / http://localhost:14004/ nocanon
ProxyPassReverse / http://localhost:14004/ nocanon

Include /etc/letsencrypt/options-ssl-apache.conf

# EDIT THESE:
SSLCertificateFile /etc/letsencrypt/live/conduit.koesters.xyz/fullchain.pem
SSLCertificateKeyFile /etc/letsencrypt/live/conduit.koesters.xyz/privkey.pem
</VirtualHost>
```

Then run
```bash
$ sudo systemctl reload apache2
```


## SSL Certificate

The easiest way to get an SSL certificate for the domain is to install `certbot` and run this:
```bash
$ sudo certbot -d conduit.koesters.xyz
```


## You're done!

Now you can start Conduit with
```bash
$ sudo systemctl start conduit
```

and set it to start automatically when your system boots with
```bash
$ sudo systemctl enable conduit
```
