# conduwuit for FreeBSD

conduwuit at the moment does not provide FreeBSD builds. Building conduwuit on FreeBSD requires a specific environment variable to use the
system prebuilt RocksDB library instead of rust-rocksdb / rust-librocksdb-sys which does *not* work and will cause a build error or coredump.

Use the following environment variable: `ROCKSDB_LIB_DIR=/usr/local/lib`

Such example commandline with it can be: `ROCKSDB_LIB_DIR=/usr/local/lib cargo build --release`
