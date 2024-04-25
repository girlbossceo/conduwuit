#### **Note: This list may not up to date. There are rapidly more and more improvements, fixes, changes, etc being made that it is becoming more difficult to maintain this list. I recommend that you give conduwuit a try and see the differences for yourself. If you have any concerns, feel free to join the conduwuit Matrix room and ask any pre-usage questions.**

### list of features, bug fixes, etc that conduwuit does that Conduit does not:

Outgoing typing indicators, outgoing read receipts, **and** outgoing presence!

## Performance:
- Concurrency support for key fetching for faster remote room joins and room joins that will error less frequently
- Send `Cache-Control` response header with `immutable` and 1 year cache length for all media requests to instruct clients to cache media, and reduce server load from media requests that could be otherwise cached
- Add feature flags and config options to enable/build with zstd, brotli, and/or gzip HTTP body compression (response and request)
- Eliminate all usage of the thread-blocking `getaddrinfo(3)` call upon DNS queries, significantly improving federation latency/ping and cache DNS results (NXDOMAINs, successful queries, etc) using hickory-dns / hickory-resolver
- Vastly improve RocksDB default settings to use new features that help with performance significantly, uses settings tailored to SSDs, various ways to tweak RocksDB, and a conduwuit setting to tell RocksDB to use settings that are tailored to HDDs or slow spinning rust storage or buggy filesystems.
- Add a Cargo build profile for aggressive build-time performance optimisations for release builds (1 codegen unit, no debug, fat LTO, etc, and optimise all crates with same)
- Implement database flush and cleanup conduwuit operations when using RocksDB
- Implement RocksDB write buffer corking and coalescing in database write-heavy areas
- Perform connection pooling and keepalives where necessary to significantly improve federation performance and latency
- Various config options to tweak connection pooling, request timeouts, connection timeouts, DNS timeouts and settings, etc with good defaults which also help huge with performance via reusing connections and retrying where needed
- Implement building conduwuit with jemalloc (which extends to the RocksDB jemalloc feature for maximum gains) or hardened_malloc light variant, and produce CI builds with jemalloc for performance (Nix doesn't seem to build [hardened_malloc-rs](https://github.com/girlbossceo/hardened_malloc-rs) properly)
- Add support for caching DNS results with hickory-dns / hickory-resolver in conduwuit (not a replacement for a proper resolver cache, but still far better than nothing)
- Overall significant database, Client-Server, and federation performance and latency improvements (check out the ping room leaderboards if you don't believe me :>)
- Add config options for RocksDB compression and bottommost compression, including choosing the algorithm and compression level


## General Fixes:
- Raise and improve all the various request timeouts making some things like room joins and client bugs error less or none at all than they should, and make them all user configurable
- Add missing `reason` field to user ban events (`/ban`)
- Fixed spec compliance issue with room version 8 - 11 joins (https://github.com/matrix-org/synapse/issues/16717 / https://github.com/matrix-org/matrix-spec/issues/1708)
- Safer and cleaner shutdowns on both database side as we run cleanup on shutdown and exits database loop better (no potential hanging issues in database loop), overall cleaner shutdown logic
- Stop sending `make_join` requests on room joins if 15 servers respond with `M_UNSUPPORTED_ROOM_VERSION` or `M_INVALID_ROOM_VERSION`
- Stop sending `make_join` requests if 50 servers cannot provide `make_join` for us
- Respect *most* client parameters for `/media/` requests (`allow_redirect` still needs work)
- Increased graceful shutdown timeout from a low 60 seconds to 180 seconds to avoid killing connections and let the remaining ones finish processing
- Return joined member count of rooms for push rules/conditions instead of a hardcoded value of 10
- Make `CONDUIT_CONFIG` optional, relevant for container users that configure only by environment variables and no longer need to set `CONDUIT_CONFIG` to an empty string.
- Allow HEAD HTTP requests in CORS for clients (despite not being explicity mentioned in Matrix spec, HTTP spec says all HEAD requests need to behave the same as GET requests, Synapse supports HEAD requests)
- Add missing `destination` key to all `X-Matrix` `Authorization` requests (spec compliance issue)


## Moderation:
- (Also see [Admin Room](#admin-room) for all the admin commands pertaining to moderation, there's a lot!)
- Add support for serving `support` well-known from `[well_known.support]` (MSC1929)
- Config option to forbid publishing rooms to the room directory (`lockdown_public_room_directory`) except for admins
- Admin commands to delete room aliases and unpublish rooms from our room directory
- For all [`/report`](https://spec.matrix.org/v1.9/client-server-api/#post_matrixclientv3roomsroomidreporteventid) requests: check if the reported event ID belongs to the reported room ID, raise report reasoning character limit to 750, fix broken formatting, make a small delayed random response per spec suggestion on privacy, and check if the sender user is in the reported room.
- Support blocking servers from downloading remote media from, returning a 404
- Don't allow `m.call.invite` events to be sent in public rooms (prevents calling the entire room)
- On new public room creations, only allow moderators to send `m.call.invite`, `org.matrix.msc3401.call`, and `org.matrix.msc3401.call.member` events
- Add support for a "global ACLs" feature (`forbidden_remote_server_names`) that blocks inbound remote room invites, room joins by room ID on server name, room joins by room alias on server name, incoming federated joins, and incoming federated room directory requests. This is very helpful for blocking servers that are purely toxic/bad and serve no value in allowing our users to suffer from things like room invite spam or such. Please note that this is not a substitute for room ACLs.
- Add support for a config option to forbid our local users from sending federated room directory requests for (`forbidden_remote_room_directory_server_names`). Similar to above, useful for blocking servers that help prevent our users from wandering into bad areas of Matrix via room directories of those malicious servers.


## Privacy/Security:
- Add config option for device name federation with a privacy-friendly default (disabled)
- Add config option for requiring authentication to the `/publicRooms` endpoint (room directory) with a default enabled for privacy
- Add config option for federating `/publicRooms` endpoint (room directory) to other servers with a default disabled for privacy
- Uses proper `argon2` crate by RustCrypto instead of questionable `rust-argon2` crate
- Generate passwords with 25 characters instead of 15
- Config option `ip_range_denylist` to support refusing to send requests (typically federation) to specific IP ranges, typically RFC 1918, non-routable, testnet, etc addresses like Synapse for security (note: this is not a guaranteed protection, and you should be using a firewall with zones if you want guaranteed protection as doing this on the application level is prone to bypasses).
- Config option to block non-admin users from sending room invites or receiving remote room invites. Admin users are still allowed.
- Config option to disable incoming and/or outgoing remote read receipts
- Config option to disable incoming and/or outgoing remote typing indicators
- Config option to disable incoming, outgoing, and/or local presence


## Administration/Logging:
- Commandline argument to specify the path to a config file instead of relying on `CONDUIT_CONFIG`
- Revamped admin room infrastructure and commands
- Substantially clean up, improve, and fix logging (less noisy dead server logging, registration attempts, more useful troubleshooting logging, proper error propagation, etc)
- Configurable RocksDB logging (`LOG` files) with proper defaults (rotate, max size, verbosity, etc) to stop LOG files from accumulating so much
- Explicit startup error if your configuration allows open registration without a token or such like Synapse with a way to bypass it if needed
- Replace the lightning bolt emoji option with support for setting any arbitrary text (e.g. another emoji) to suffix to all new user registrations, with a conduwuit default of 🏳️‍⚧️
- Implement config option to auto join rooms upon registration
- Warn on unknown config options specified
- Add `/_conduwuit/server_version` route to return the version of conduwuit without relying on the federation API `/_matrix/federation/v1/version`
- Add configurable RocksDB recovery modes to aid in recovering corrupted RocksDB databases
- Support config options via `CONDUWUIT_` prefix
- Add support for listening on multiple TCP ports
- Disable update check by default as it's not useful for conduwuit
- **Opt-in** Sentry.io telemetry and metrics, mainly used for crash reporting


## Maintenance/Stability:
- GitLab CI ported to GitHub Actions
- Extensively revamp the example config to be extremely helpful and useful to both new users and power users
- Fixed every single clippy (default lints) and rustc warnings, including some that were performance related or potential safety issues / unsoundness
- Add a **lot** of other clippy and rustc lints and a rustfmt.toml file
- Has [Renovate](https://docs.renovatebot.com/), [Trivy](https://github.com/aquasecurity/trivy-action), and keeps ALL dependencies as up to date as possible
- Attempts and interest in removing extreme and unnecessary panics/unwraps/expects that can lead to denial of service or such (upstream and upstream contributors want this unusual behaviour for some reason)
- Purge unmaintained/irrelevant/broken database backends (heed, sled, persy) and other unnecessary code or overhead
- webp support for images
- Add cargo audit support to CI
- Add timestamp by commit date support to building OCI images for keeping image build reproducibility and still have a meaningful "last modified date" for OCI image metadata
- Update rusqlite/sqlite (not that you should be using it)
- Startup check if conduwuit running in a container and is listening on 127.0.0.1
- Bump


## Admin Room:
- Fix admin room handler to not panic/crash if the admin room command response fails (e.g. too large message)
- Add command to dynamically change conduwuit's tracing log level filter on the fly
- Add admin command to fetch a server's `/.well-known/matrix/support` file
- Add debug admin command to force update user device lists (could potentially resolve some E2EE flukes)
- Implement **RocksDB online backups**, listing RocksDB backups, and listing database file counts all via admin commands
- Add various database visibility commands such as being able to query the getters and iterators used in conduwuit, a very helpful online debugging utility
- Forbid the admin room from being made public or world readable history
- Add `!admin` as a way to call the admin bot
- Extend clear cache admin command to support clearing more caches such as DNS and TLS name overrides
- Admin debug command to send a federation request/ping to a server's `/_matrix/federation/v1/version` endpoint and measures the latency it took
- Add admin command to bulk delete media via a codeblock list of MXC URLs.
- Add admin command to delete both the thumbnail and media MXC URLs from an event ID (e.g. from an abuse report)
- Add admin command to list all the rooms a local user is joined in
- Add admin command to delete all remote media in the past X minutes as a form of deleting media that you don't want on your server that a remote user posted in a room
- Add admin command to return a room's state
- Admin debug command to fetch a PDU from a remote server and inserts it into our database/timeline as backfill
- Add admin command to delete media via a specific MXC. This deletes the MXC from our database, and the file locally.
- Add admin commands for banning (blocking) room IDs from our local users joining (admins are always allowed) and evicts all our local users from that room, in addition to bulk room banning support, and blocks room invites (remote and local) to the banned room, as a moderation feature


## Misc:
- Support for creating rooms with custom room IDs like Maunium Synapse (`room_id` request body field to `/createRoom`)
- Query parameter `?format=event|content` for returning either the room state event's content (default) for the full room state event on ` /_matrix/client/v3/rooms/{roomId}/state/{eventType}[/{stateKey}]` requests (see https://github.com/matrix-org/matrix-spec/issues/1047)
- Support for suggesting servers to join at `/_matrix/client/v3/directory/room/{roomAlias}`
- Add **optional** feature flag to use SHA256 key names for media instead of base64 to overcome filesystem file name length limitations (OS error file name too long)
- Send a User-Agent on all of our requests
- Send `avatar_url` on invite room membership events/changes
- Support sending [`well_known` response to client login responses](https://spec.matrix.org/v1.10/client-server-api/#post_matrixclientv3login) if using config option `[well_known.client]`
- Implement `include_state` search criteria support for `/search` requests (response now can include room states)
- Declare various missing Matrix versions and features at `/_matrix/client/versions`
- Implement legacy Matrix `/v1/` media endpoints that some clients and servers may still call
- Config option to change Conduit's behaviour of homeserver key fetching (`query_trusted_key_servers_first`). This option sets whether conduwuit will query trusted notary key servers first before the individual homeserver(s), or vice versa which may help in joining certain rooms.
- Implement unstable MSC2666 support for querying mutual rooms with a user
- Assume well-knowns are broken if they exceed past 10000 characters.
- Add support for the Matrix spec compliance test suite [Complement](https://github.com/matrix-org/complement/)
- Add support for listening on both HTTP and HTTPS if using direct TLS with conduwuit for usecases such as Complement
- No cryptocurrency donations allowed, conduwuit is fully maintained by independent queer maintainers, and with a strong priority on inclusitivity and comfort for protected groups 🏳️‍⚧️
