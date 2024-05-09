# Maintaining your conduwuit setup

## Moderation

conduwuit has moderation through admin room commands. "binary commands" (medium priority) and an admin API (low priority) is planned. Some moderation-related config options are available in the example config such as "global ACLs" and blocking media requests to certain servers. See the example config for the moderation config options under the "Moderation / Privacy / Security" section.

conduwuit has moderation admin commands for:
- managing room aliases (`!admin rooms alias`)
- managing room directory (`!admin rooms directory`)
- managing room banning/blocking and user removal (`!admin rooms moderation`)
- managing user accounts (`!admin users`)
- fetching `/.well-known/matrix/support` from servers (`!admin federation`)
- blocking incoming federation for certain rooms (not the same as room banning) (`!admin federation`)
- deleting media (see [the media section](#media))

Any commands with `-list` in them will require a codeblock in the message with each object being newline delimited. An example of doing this is:

````
!admin rooms moderation ban-list-of-rooms
```
!roomid1:server.name
!roomid2:server.name
!roomid3:server.name
```
````

## Database

If using RocksDB, there's very little you need to do. Compaction is ran automatically based on various defined thresholds tuned for conduwuit to be high performance with the least I/O amplifcation or overhead. Manually running compaction is not recommended, or compaction via a timer. RocksDB is built with io_uring support via liburing for async read I/O.

Some RocksDB settings can be adjusted such as the compression method chosen. See the RocksDB section in the [example config](configuration.md). btrfs users may benefit from disabling compression on RocksDB if CoW is in use.

RocksDB troubleshooting can be found [in the RocksDB section of troubleshooting](troubleshooting.md).

## Backups

Currently only RocksDB supports online backups. If you'd like to backup your database online without any downtime, see the `!admin server` command for the backup commands and the `database_backup_path` config options in the example config. Please note that the format of the database backup is not the exact same. This is unfortunately a bad design choice by Facebook as we are using the database backup engine API from RocksDB, however the data is still there and can still be joined together.

To restore a backup from an online RocksDB backup:
- shutdown conduwuit
- create a new directory for merging together the data
- in the online backup created, copy all `.sst` files in `$DATABASE_BACKUP_PATH/shared_checksum` to your new directory
- trim all the strings so instead of `######_sxxxxxxxxx.sst`, it reads `######.sst`. A way of doing this with sed and bash is `for file in *.sst; do mv "$file" "$(echo "$file" | sed 's/_s.*/.sst/')"; done`
- copy all the files in `$DATABASE_BACKUP_PATH/1` to your new directory
- set your `database_path` config option to your new directory, or replace your old one with the new one you crafted
- start up conduwuit again and it should open as normal

If you'd like to do an offline backup, shutdown conduwuit and copy your `database_path` directory elsewhere. This can be restored with no modifications needed.

Backing up media is also just copying the `media/` directory from your database directory.

## Media

Media still needs various work, however conduwuit implements media deletion via:
- MXC URI
- Delete list of MXC URIs
- Delete remote media in the past `N` seconds/minutes

See the `!admin media` command for further information. All media in conduwuit is stored at `$DATABASE_DIR/media`. This will be configurable soon.

If you are finding yourself needing extensive granular control over media, we recommend looking into [Matrix Media Repo](https://github.com/t2bot/matrix-media-repo). conduwuit intends to implement various utilities for media, but MMR is dedicated to extensive media management.

Built-in S3 support is also planned, but for now using a "S3 filesystem" on `media/` works. conduwuit also sends a `Cache-Control` header of 1 year and immutable for all media requests (download and thumbnail) to reduce unnecessary media requests from browsers.
