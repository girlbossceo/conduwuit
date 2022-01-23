## Cross compilation

The `cross` folder contains a set of convenience scripts (`build.sh` and `test.sh`) for cross-compiling Conduit.

Currently supported targets are

- aarch64-unknown-linux-musl
- arm-unknown-linux-musleabihf
- armv7-unknown-linux-musleabihf
- x86\_64-unknown-linux-musl

### Install prerequisites
#### Docker
[Installation guide](https://docs.docker.com/get-docker/).
```sh
$ sudo apt install docker
$ sudo systemctl start docker
$ sudo usermod -aG docker $USER
$ newgrp docker
```

#### Cross
[Installation guide](https://github.com/rust-embedded/cross/#installation).
```sh
$ cargo install cross
```

### Buiding Conduit
```sh
$ TARGET=armv7-unknown-linux-musleabihf ./cross/build.sh --release
```
The cross-compiled binary is at `target/armv7-unknown-linux-musleabihf/release/conduit`

### Testing Conduit
```sh
$ TARGET=armv7-unknown-linux-musleabihf ./cross/test.sh --release
```
