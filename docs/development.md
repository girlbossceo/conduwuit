# Development

Information about developing the project. If you are only interested in using
it, you can safely ignore this section. If you plan on contributing, see the
[contributor's guide](contributing.md).

## List of forked dependencies During conduwuit development, we have had to fork
some dependencies to support our use-cases in some areas. This ranges from
things said upstream project won't accept for any reason, faster-paced
development (unresponsive or slow upstream), conduwuit-specific usecases, or
lack of time to upstream some things.

- [ruma/ruma][1]: <https://github.com/girlbossceo/ruwuma> - various performance
improvements, more features, faster-paced development, client/server interop
hacks upstream won't accept, etc
- [facebook/rocksdb][2]: <https://github.com/girlbossceo/rocksdb> - liburing
build fixes, GCC build fix, and logging callback C API for Rust tracing
integration
- [tikv/jemallocator][3]: <https://github.com/girlbossceo/jemallocator> - musl
builds seem to be broken on upstream
- [zyansheep/rustyline-async][4]:
<https://github.com/girlbossceo/rustyline-async> - tab completion callback and
`CTRL+\` signal quit event for CLI
- [rust-rocksdb/rust-rocksdb][5]:
<https://github.com/girlbossceo/rust-rocksdb-zaidoon1> - [`@zaidoon1`'s][8] fork
has quicker updates, more up to date dependencies. Our changes fix musl build
issues, Rust part of the logging callback C API, removes unnecessary `gtest`
include, and uses our RocksDB and jemallocator
- [tokio-rs/tracing][6]: <https://github.com/girlbossceo/tracing> - Implements
`Clone` for `EnvFilter` to support dynamically changing tracing envfilter's
alongside other logging/metrics things

## Debugging with `tokio-console`

[`tokio-console`][7] can be a useful tool for debugging and profiling. To make a
`tokio-console`-enabled build of conduwuit, enable the `tokio_console` feature,
disable the default `release_max_log_level` feature, and set the `--cfg
tokio_unstable` flag to enable experimental tokio APIs. A build might look like
this:

```bash RUSTFLAGS="--cfg tokio_unstable" cargo build \ --release \
--no-default-features \
--features=systemd,element_hacks,gzip_compression,brotli_compression,zstd_compression,tokio_console
```

[1]: https://github.com/ruma/ruma/
[2]: https://github.com/facebook/rocksdb/
[3]: https://github.com/tikv/jemallocator/
[4]: https://github.com/zyansheep/rustyline-async/
[5]: https://github.com/rust-rocksdb/rust-rocksdb/
[6]: https://github.com/tokio-rs/tracing/
[7]: https://docs.rs/tokio-console/latest/tokio_console/
[8]: https://github.com/zaidoon1/
