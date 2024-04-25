# Development

Information about developing the project. If you are only interested in using
it, you can safely ignore this section.

## Debugging with `tokio-console`

[`tokio-console`][1] can be a useful tool for debugging and profiling. To make
a `tokio-console`-enabled build of Conduwuit, enable the `tokio_console` feature,
disable the default `release_max_log_level` feature, and set the
`--cfg tokio_unstable` flag to enable experimental tokio APIs. A build might
look like this:

```bash
RUSTFLAGS="--cfg tokio_unstable" cargo build \
    --release \
    --no-default-features \
    --features
    backend_rocksdb,systemd,element_hacks,sentry_telemetry,gzip_compression,brotli_compression,zstd_compression,tokio_console
```

[1]: https://docs.rs/tokio-console/latest/tokio_console/
