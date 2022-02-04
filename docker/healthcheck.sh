#!/bin/sh

# If the config file does not contain a default port and the CONDUIT_PORT env is not set, create
# try to get port from process list
if [ -z "${CONDUIT_PORT}" ]; then
  CONDUIT_PORT=$(ss -tlpn | grep conduit | grep -m1 -o ':[0-9]*' | grep -m1 -o '[0-9]*')
fi

# The actual health check.
# We try to first get a response on HTTP and when that fails on HTTPS and when that fails, we exit with code 1.
# TODO: Change this to a single wget call. Do we have a config value that we can check for that?
wget --no-verbose --tries=1 --spider "http://localhost:${CONDUIT_PORT}/_matrix/client/versions" || \
    wget --no-verbose --tries=1 --spider "https://localhost:${CONDUIT_PORT}/_matrix/client/versions" || \
    exit 1
