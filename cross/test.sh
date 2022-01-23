#!/usr/bin/env sh
set -ex

# Build conduit for a specific target
cross/build.sh $@

# Test conduit for a specific target
cross test --target="$TARGET" $@
