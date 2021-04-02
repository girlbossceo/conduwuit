Install docker:
```
$ sudo apt install docker
$ sudo usermod -aG docker $USER
$ exec sudo su -l $USER
$ sudo systemctl start docker
$ cargo install cross
$ cross build --release --features tls_vendored --target armv7-unknown-linux-musleabihf
```
The cross-compiled binary is at target/armv7-unknown-linux-musleabihf/release/conduit
