#### **Note: This list may not up to date. There are rapidly more and more
improvements, fixes, changes, etc being made that it is becoming more difficult
to maintain this list. I recommend that you give conduwuit a try and see the
differences for yourself. If you have any concerns, feel free to join the
conduwuit Matrix room and ask any pre-usage questions.**

### list of features, bug fixes, etc that conduwuit does that Conduit does not

Outgoing typing indicators, outgoing read receipts, **and** outgoing presence!

## Performance

- Concurrency support for individual homeserver key fetching for faster remote
room joins and room joins that will error less frequently
- Send `Cache-Control` response header with `immutable` and 1 year cache length
for all media requests (download and thumbnail) to instruct clients to cache
media, and reduce server load from media requests that could be otherwise cached
- Add feature flags and config options to enable/build with zstd, brotli, and/or
gzip HTTP body compression (response and request)
- Eliminate all usage of the thread-blocking `getaddrinfo(3)` call upon DNS
queries, significantly improving federation latency/ping and cache DNS results
(NXDOMAINs, successful queries, etc) using hickory-dns / hickory-resolver
- Enable HTTP/2 support on all requests
- Vastly improve RocksDB default settings to use new features that help with
performance significantly, uses settings tailored to SSDs, various ways to tweak
RocksDB, and a conduwuit setting to tell RocksDB to use settings that are
tailored to HDDs or slow spinning rust storage or buggy filesystems.
- Implement database flush and cleanup conduwuit operations when using RocksDB
- Implement RocksDB write buffer corking and coalescing in database write-heavy
areas
- Perform connection pooling and keepalives where necessary to significantly
improve federation performance and latency
- Various config options to tweak connection pooling, request timeouts,
connection timeouts, DNS timeouts and settings, etc with good defaults which
also help huge with performance via reusing connections and retrying where
needed
- Properly get and use the amount of parallelism / tokio workers
- Implement building conduwuit with jemalloc (which extends to the RocksDB
jemalloc feature for maximum gains) or hardened_malloc light variant, and
io_uring support, and produce CI builds with jemalloc and io_uring by default
for performance (Nix doesn't seem to build
[hardened_malloc-rs](https://github.com/girlbossceo/hardened_malloc-rs)
properly)
- Add support for caching DNS results with hickory-dns / hickory-resolver in
conduwuit (not a replacement for a proper resolver cache, but still far better
than nothing), also properly falls back on TCP for UDP errors or if a SRV
response is too large
- Add config option for using DNS over TCP, and config option for controlling
A/AAAA record lookup strategy (e.g. don't query AAAA records if you don't have
IPv6 connectivity)
- Overall significant database, Client-Server, and federation performance and
latency improvements (check out the ping room leaderboards if you don't believe
me :>)
- Add config options for RocksDB compression and bottommost compression,
including choosing the algorithm and compression level
- Use [loole](https://github.com/mahdi-shojaee/loole) MPSC channels instead of
tokio MPSC channels for huge performance boosts in sending channels (mainly
relevant for federation) and presence channels
- Use `tracing`/`log`'s `release_max_level_info` feature to improve performance,
build speeds, binary size, and CPU usage in release builds by avoid compiling
debug/trace log level macros that users will generally never use (can be
disabled with a build-time feature flag)
- Remove some unnecessary checks on EDU handling for incoming transactions,
effectively speeding them up
- Simplify, dedupe, etc huge chunks of the codebase, including some that were
unnecessary overhead, binary bloats, or preventing compiler/linker optimisations
- Implement zero-copy RocksDB database accessors, substantially improving
performance caused by unnecessary memory allocations

## General Fixes/Features

- Add legacy Element client hack fixing password changes and deactivations on
legacy Element Android/iOS due to usage of an unspecced `user` field for UIAA
- Raise and improve all the various request timeouts making some things like
room joins and client bugs error less or none at all than they should, and make
them all user configurable
- Add missing `reason` field to user ban events (`/ban`)
- Safer and cleaner shutdowns across incoming/outgoing requests (graceful
shutdown) and the database
- Stop sending `make_join` requests on room joins if 15 servers respond with
`M_UNSUPPORTED_ROOM_VERSION` or `M_INVALID_ROOM_VERSION`
- Stop sending `make_join` requests if 50 servers cannot provide `make_join` for
us
- Respect *most* client parameters for `/media/` requests (`allow_redirect`
still needs work)
- Return joined member count of rooms for push rules/conditions instead of a
hardcoded value of 10
- Make `CONDUIT_CONFIG` optional, relevant for container users that configure
only by environment variables and no longer need to set `CONDUIT_CONFIG` to an
empty string.
- Allow HEAD and PATCH (MSC4138) HTTP requests in CORS for clients (despite not
being explicity mentioned in Matrix spec, HTTP spec says all HEAD requests need
to behave the same as GET requests, Synapse supports HEAD requests)
- Fix using conduwuit with flake-compat on NixOS
- Resolve and remove some "features" from upstream that result in concurrency
hazards, exponential backoff issues, or arbitrary performance limiters
- Find more servers for outbound federation `/hierarchy` requests instead of
just the room ID server name
- Support for suggesting servers to join through at
`/_matrix/client/v3/directory/room/{roomAlias}`
- Support for suggesting servers to join through us at
`/_matrix/federation/v1/query/directory`
- Misc edge-case search fixes (e.g. potentially missing some events)
- Misc `/sync` fixes (e.g. returning unnecessary data or incorrect/invalid
responses)
- Add `replaces_state` and `prev_sender` in `unsigned` for state event changes
which primarily makes Element's "See history" button on a state event functional
- Fix Conduit not allowing incoming federation requests for various world
readable rooms
- Fix Conduit not respecting the client-requested file name on media requests
- Prevent sending junk / non-membership events to `/send_join` and `/send_leave`
endpoints
- Only allow the requested membership type on `/send_join` and `/send_leave`
endpoints (e.g. don't allow leave memberships on join endpoints)
- Prevent state key impersonation on `/send_join` and `/send_leave` endpoints
- Validate `X-Matrix` origin and request body `"origin"` field on incoming
transactions
- Add `GET /_matrix/client/v1/register/m.login.registration_token/validity`
endpoint
- Explicitly define support for sliding sync at `/_matrix/client/versions`
(`org.matrix.msc3575`)
- Fix seeing empty status messages on user presences

## Moderation

- (Also see [Admin Room](#admin-room) for all the admin commands pertaining to
moderation, there's a lot!)
- Add support for room banning/blocking by ID using admin command
- Add support for serving `support` well-known from `[global.well_known]`
(MSC1929) (`/.well-known/matrix/support`)
- Config option to forbid publishing rooms to the room directory
(`lockdown_public_room_directory`) except for admins
- Admin commands to delete room aliases and unpublish rooms from our room
directory
- For all
[`/report`](https://spec.matrix.org/latest/client-server-api/#post_matrixclientv3roomsroomidreporteventid)
requests: check if the reported event ID belongs to the reported room ID, raise
report reasoning character limit to 750, fix broken formatting, make a small
delayed random response per spec suggestion on privacy, and check if the sender
user is in the reported room.
- Support blocking servers from downloading remote media from, returning a 404
- Don't allow `m.call.invite` events to be sent in public rooms (prevents
calling the entire room)
- On new public room creations, only allow moderators to send `m.call.invite`,
`org.matrix.msc3401.call`, and `org.matrix.msc3401.call.member` events to
prevent unprivileged users from calling the entire room
- Add support for a "global ACLs" feature (`forbidden_remote_server_names`) that
blocks inbound remote room invites, room joins by room ID on server name, room
joins by room alias on server name, incoming federated joins, and incoming
federated room directory requests. This is very helpful for blocking servers
that are purely toxic/bad and serve no value in allowing our users to suffer
from things like room invite spam or such. Please note that this is not a
substitute for room ACLs.
- Add support for a config option to forbid our local users from sending
federated room directory requests for
(`forbidden_remote_room_directory_server_names`). Similar to above, useful for
blocking servers that help prevent our users from wandering into bad areas of
Matrix via room directories of those malicious servers.
- Add config option for auto remediating/deactivating local non-admin users who
attempt to join bad/forbidden rooms (`auto_deactivate_banned_room_attempts`)
- Deactivating users will remove their profile picture, blurhash, display name,
and leave all rooms by default just like Synapse and for additional privacy
- Reject some EDUs from ACL'd users such as read receipts and typing indicators

## Privacy/Security

- Add config option for device name federation with a privacy-friendly default
(disabled)
- Add config option for requiring authentication to the `/publicRooms` endpoint
(room directory) with a default enabled for privacy
- Add config option for federating `/publicRooms` endpoint (room directory) to
other servers with a default disabled for privacy
- Uses proper `argon2` crate by RustCrypto instead of questionable `rust-argon2`
crate
- Generate passwords with 25 characters instead of 15
- Config option `ip_range_denylist` to support refusing to send requests
(typically federation) to specific IP ranges, typically RFC 1918, non-routable,
testnet, etc addresses like Synapse for security (note: this is not a guaranteed
protection, and you should be using a firewall with zones if you want guaranteed
protection as doing this on the application level is prone to bypasses).
- Config option to block non-admin users from sending room invites or receiving
remote room invites. Admin users are still allowed.
- Config option to disable incoming and/or outgoing remote read receipts
- Config option to disable incoming and/or outgoing remote typing indicators
- Config option to disable incoming, outgoing, and/or local presence and for
timing out remote users
- Sanitise file names for the `Content-Disposition` header for all media
requests (thumbnails, downloads, uploads)
- Media repository on handling `Content-Disposition` and `Content-Type` is fully
spec compliant and secured
- Send secure default HTTP headers such as a strong restrictive CSP (see
MSC4149), deny iframes, disable `X-XSS-Protection`, disable interest cohort in
`Permission-Policy`, etc to mitigate any potential attack surface such as from
untrusted media

## Administration/Logging

- Commandline argument to specify the path to a config file instead of relying
on `CONDUIT_CONFIG`
- Revamped admin room infrastructure and commands
- Substantially clean up, improve, and fix logging (less noisy dead server
logging, registration attempts, more useful troubleshooting logging, proper
error propagation, etc)
- Configurable RocksDB logging (`LOG` files) with proper defaults (rotate, max
size, verbosity, etc) to stop LOG files from accumulating so much
- Explicit startup error if your configuration allows open registration without
a token or such like Synapse with a way to bypass it if needed
- Replace the lightning bolt emoji option with support for setting any arbitrary
text (e.g. another emoji) to suffix to all new user registrations, with a
conduwuit default of "🏳️‍⚧️"
- Implement config option to auto join rooms upon registration
- Warn on unknown config options specified
- Add `/_conduwuit/server_version` route to return the version of conduwuit
without relying on the federation API `/_matrix/federation/v1/version`
- Add `/_conduwuit/local_user_count` route to return the amount of registered
active local users on your homeserver *if federation is enabled*
- Add configurable RocksDB recovery modes to aid in recovering corrupted RocksDB
databases
- Support config options via `CONDUWUIT_` prefix and accessing non-global struct
config options with the `__` split (e.g. `CONDUWUIT_WELL_KNOWN__SERVER`)
- Add support for listening on multiple TCP ports and multiple addresses
- **Opt-in** Sentry.io telemetry and metrics, mainly used for crash reporting
- Log the client IP on various requests such as registrations, banned room join
attempts, logins, deactivations, federation transactions, etc
- Fix Conduit dropping some remote server federation response errors

## Maintenance/Stability

- GitLab CI ported to GitHub Actions
- Add support for the Matrix spec compliance test suite
[Complement](https://github.com/matrix-org/complement/) via the Nix flake and
various other fixes for it
- Implement running and diff'ing Complement results in CI and error if any
mismatch occurs to prevent large cases of conduwuit regressions
- Repo is (officially) mirrored to GitHub, GitLab, git.gay, git.girlcock.ceo,
sourcehut, and Codeberg (see README.md for their links)
- Docker container images published to GitLab Container Registry, GitHub
Container Registry, and Dockerhub
- Extensively revamp the example config to be extremely helpful and useful to
both new users and power users
- Fixed every single clippy (default lints) and rustc warnings, including some
that were performance related or potential safety issues / unsoundness
- Add a **lot** of other clippy and rustc lints and a rustfmt.toml file
- Repo uses [Renovate](https://docs.renovatebot.com/) and keeps ALL
dependencies as up to date as possible
- Purge unmaintained/irrelevant/broken database backends (heed, sled, persy) and
other unnecessary code or overhead
- webp support for images
- Add cargo audit support to CI
- Add documentation lints via lychee and markdownlint-cli to CI
- CI tests for all sorts of feature matrixes (jemalloc, non-defaullt, all
features, etc)
- Add static and dynamic linking smoke tests in CI to prevent any potential
linking regressions for Complement, static binaries, Nix devshells, etc
- Add timestamp by commit date when building OCI images for keeping image build
reproducibility and still have a meaningful "last modified date" for OCI image
- Add timestamp by commit date via `SOURCE_DATE_EPOCH` for Debian packages
- Startup check if conduwuit running in a container and is listening on
127.0.0.1 (generally containers are using NAT networking and 0.0.0.0 is the
intended listening address)
- Add a panic catcher layer to return panic messages in HTTP responses if a
panic occurs
- Add full compatibility support for SHA256 media file names instead of base64
file names to overcome filesystem file name length limitations (OS error file
name too long) while still retaining upstream database compatibility
- Remove SQLite support due to being very poor performance, difficult to
maintain against RocksDB, and is a blocker to significantly improved database
code

## Admin Room

- Add support for a console CLI interface that can issue admin commands and
output them in your terminal
- Add support for an admin-user-only commandline admin room interface that can
be issued in any room with the `\\!admin` or `\!admin` prefix and returns the
response as yourself in the same room
- Add admin commands for uptime, server startup, server shutdown, and server
restart
- Fix admin room handler to not panic/crash if the admin room command response
fails (e.g. too large message)
- Add command to dynamically change conduwuit's tracing log level filter on the
fly
- Add admin command to fetch a server's `/.well-known/matrix/support` file
- Add debug admin command to force update user device lists (could potentially
resolve some E2EE flukes)
- Implement **RocksDB online backups**, listing RocksDB backups, and listing
database file counts all via admin commands
- Add various database visibility commands such as being able to query the
getters and iterators used in conduwuit, a very helpful online debugging utility
- Forbid the admin room from being made public or world readable history
- Add `!admin` as a way to call the admin bot
- Extend clear cache admin command to support clearing more caches such as DNS
and TLS name overrides
- Admin debug command to send a federation request/ping to a server's
`/_matrix/federation/v1/version` endpoint and measures the latency it took
- Add admin command to bulk delete media via a codeblock list of MXC URLs.
- Add admin command to delete both the thumbnail and media MXC URLs from an
event ID (e.g. from an abuse report)
- Add admin command to list all the rooms a local user is joined in
- Add admin command to list joined members in a room
- Add admin command to view the room topic of a room
- Add admin command to delete all remote media in the past X minutes as a form
of deleting media that you don't want on your server that a remote user posted
in a room, a `--force` flag to ignore errors, and support for reading `last
modified time` instead of `creation time` for filesystems that don't support
file created metadata
- Add admin command to return a room's full/complete state
- Admin debug command to fetch a PDU from a remote server and inserts it into
our database/timeline as backfill
- Add admin command to delete media via a specific MXC. This deletes the MXC
from our database, and the file locally.
- Add admin commands for banning (blocking) room IDs from our local users
joining (admins are always allowed) and evicts all our local users from that
room, in addition to bulk room banning support, and blocks room invites (remote
and local) to the banned room, as a moderation feature
- Add admin commands to output jemalloc memory stats and memory usage
- Add admin command to get rooms a *remote* user shares with us
- Add debug admin commands to get the earliest and latest PDU in a room
- Add debug admin command to echo a message
- Add admin command to insert rooms tags for a user, most useful for inserting
the `m.server_notice` tag on your admin room to make it "persistent" in the
"System Alerts" section of Element
- Add experimental admin debug command for Dendrite's `AdminDownloadState`
(`/admin/downloadState/{serverName}/{roomID}`) admin API endpoint to download
and use a remote server's room state in the room
- Disable URL previews by default in the admin room due to various command
outputs having "URLs" in them that clients may needlessly render/request
- Extend memory usage admin server command to support showing memory allocator
stats such as jemalloc's
- Add admin debug command to see memory allocator's full extended debug
statistics such as jemalloc's

## Misc

- Add guest support for accessing TURN servers via `turn_allow_guests` like
Synapse
- Support for creating rooms with custom room IDs like Maunium Synapse
(`room_id` request body field to `/createRoom`)
- Query parameter `?format=event|content` for returning either the room state
event's content (default) for the full room state event on
`/_matrix/client/v3/rooms/{roomId}/state/{eventType}[/{stateKey}]` requests (see
<https://github.com/matrix-org/matrix-spec/issues/1047>)
- Send a User-Agent on all of our requests
- Send `avatar_url` on invite room membership events/changes
- Support sending [`well_known` response to client login
responses](https://spec.matrix.org/v1.10/client-server-api/#post_matrixclientv3login)
if using config option `[well_known.client]`
- Implement `include_state` search criteria support for `/search` requests
(response now can include room states)
- Declare various missing Matrix versions and features at
`/_matrix/client/versions`
- Implement legacy Matrix `/v1/` media endpoints that some clients and servers
may still call
- Config option to change Conduit's behaviour of homeserver key fetching
(`query_trusted_key_servers_first`). This option sets whether conduwuit will
query trusted notary key servers first before the individual homeserver(s), or
vice versa which may help in joining certain rooms.
- Implement unstable MSC2666 support for querying mutual rooms with a user
- Implement unstable MSC3266 room summary API support
- Implement unstable MSC4125 support for specifying servers to join via on
federated invites
- Make conduwuit build and be functional under Nix + macOS
- Log out all sessions after unsetting the emergency password
- Assume well-knowns are broken if they exceed past 12288 characters.
- Add support for listening on both HTTP and HTTPS if using direct TLS with
conduwuit for usecases such as Complement
- Add config option for disabling RocksDB Direct IO if needed
- Add various documentation on maintaining conduwuit, using RocksDB online
backups, some troubleshooting, using admin commands, moderation documentation,
etc
- (Developers): Add support for [hot reloadable/"live" modular
development](development/hot_reload.md)
- (Developers): Add support for tokio-console
- (Developers): Add support for tracing flame graphs
- No cryptocurrency donations allowed, conduwuit is fully maintained by
independent queer maintainers, and with a strong priority on inclusitivity and
comfort for protected groups 🏳️‍⚧️
- [Add a community Code of Conduct for all conduwuit community spaces, primarily
the Matrix space](https://conduwuit.puppyirl.gay/conduwuit_coc.html)
