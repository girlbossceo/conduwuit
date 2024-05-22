# Troubleshooting conduwuit

> ## Docker users ⚠️
>
> Docker is extremely UX unfriendly. Because of this, a ton of issues or support is actually Docker support, not conduwuit support. We also cannot document the ever-growing list of Docker issues here.
>
> If you intend on asking for support and you are using Docker, **PLEASE** triple validate your issues are **NOT** because you have a misconfiguration in your Docker setup.
>
> If there are things like Compose file issues or Dockerhub image issues, those can still be mentioned as long as they're something we can fix.

## Rocksdb / database issues

#### Direct IO

Some filesystems may not like RocksDB using [Direct IO](https://github.com/facebook/rocksdb/wiki/Direct-IO). Direct IO is for non-buffered I/O which improves conduwuit performance, but at least FUSE is a filesystem potentially known to not like this. See the [example config](configuration.md) for disabling it if needed. Issues from Direct IO on unsupported filesystems are usually shown as startup errors.

#### Database corruption

If your database is corrupted and is failing to start (e.g. checksum mismatch), it may be recoverable but careful steps must be taken, and there is no guarantee it may be recoverable.

RocksDB has the following recovery modes:

- `TolerateCorruptedTailRecords`
- `AbsoluteConsistency`
- `PointInTime`
- `SkipAnyCorruptedRecord`

By default, conduwuit uses `TolerateCorruptedTailRecords` as generally these may be due to bad federation and we can re-fetch the correct data over federation. The RocksDB default is `PointInTime` which will attempt to restore a "snapshot" of the data when it was last known to be good. This data can be either a few seconds old, or multiple minutes prior. `PointInTime` may not be suitable for default usage due to clients and servers possibly not being able to handle sudden "backwards time travels", and `AbsoluteConsistency` may be too strict.

`AbsoluteConsistency` will fail to start the database if any sign of corruption is detected. `SkipAnyCorruptedRecord` will skip all forms of corruption unless it forbids the database from opening (e.g. too severe). Usage of `SkipAnyCorruptedRecord` voids any support as this may cause more damage and/or leave your database in a permanently inconsistent state, but it may do something if `PointInTime` does not work as a last ditch effort.

With this in mind:
- First start conduwuit with the `PointInTime` recovery method. See the [example config](configuration.md) for how to do this using `rocksdb_recovery_mode`
- If your database successfully opens, clients are recommended to clear their client cache to account for the rollback
- Leave your conduwuit running in `PointInTime` for at least 30-60 minutes so as much possible corruption is restored
- If all goes will, you should be able to restore back to using `TolerateCorruptedTailRecords` and you have successfully recovered your database

## Media

#### "File name too long"

If you are running into the "file name is too long" OS error for media requests, your filesystem cannot handle file name lengths >=255 characters. This is unfortuntely due to Conduit (upstream) using base64 for file name keys which is very problematic for some filesystems as the base64 input is untrusted and long file names or specific inputs can cause this. If you would like to avoid this, you may build conduwuit yourself with the `sha256_media` feature. **This will lose database compatibility with upstream**.

## Debugging

Note that users should not really be debugging things. If you find yourself debugging and find the issue, please let us know and/or how we can fix it. Various debug commands can be found in `!admin debug`.

#### Debug/Trace log level

conduwuit builds without debug or trace log levels by default for at least performance reasons. This may change in the future and/or binaries providing such configurations may be provided. If you need to access debug/trace log levels, you will need to build without the `release_max_log_level` feature.

#### Changing log level dynamically

conduwuit supports changing the tracing log environment filter on-the-fly using the admin command `!admin debug change-log-level`. This accepts a string **without quotes** the same format as the `log` config option.

#### Pinging servers

conduwuit can ping other servers using `!admin debug ping`. This takes a server name and goes through the server discovery process and queries `/_matrix/federation/v1/version`. Errors are outputted.

#### Allocator memory stats

When using jemalloc with jemallocator's `stats` feature, you can see conduwuit's jemalloc memory stats by using `!admin debug memory-stats`
