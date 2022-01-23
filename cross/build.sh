#!/bin/bash
set -ex

# build custom container with libclang and static compilation
tag="rust-cross:${TARGET:?}"
docker build --tag="$tag" - << EOF
FROM rustembedded/cross:$TARGET

# Install libclang for generating bindings with rust-bindgen
# The architecture is not relevant here since it's not used for compilation
RUN apt-get update && \
    apt-get install --assume-yes libclang-dev

# Set the target prefix
ENV TARGET_PREFIX="/usr/local/$(echo "${TARGET:?}" | sed -e 's/armv7/arm/' -e 's/-unknown//')"

# Make sure that cc-rs links libc/libstdc++ statically when cross-compiling
# See https://github.com/alexcrichton/cc-rs#external-configuration-via-environment-variables for more information
ENV RUSTFLAGS="-L\$TARGET_PREFIX/lib" CXXSTDLIB="static=stdc++"
# Forcefully linking against libatomic, libc and libgcc is required for arm32, otherwise symbols are missing
$([[ $TARGET =~ arm ]] && echo 'ENV RUSTFLAGS="$RUSTFLAGS -Clink-arg=-lgcc -Clink-arg=-latomic -lstatic=c"')
# Strip symbols while compiling in release mode
$([[ $@ =~ -r ]] && echo 'ENV RUSTFLAGS="$RUSTFLAGS -Clink-arg=-s"')

# Support a rustc wrapper like sccache when cross-compiling
ENV RUSTC_WRAPPER="$RUSTC_WRAPPER"

# Make sure that rust-bindgen uses the correct include path when cross-compiling
# See https://github.com/rust-lang/rust-bindgen#environment-variables for more information
ENV BINDGEN_EXTRA_CLANG_ARGS="-I\$TARGET_PREFIX/include"
EOF

# build conduit for a specific target
cross build --target="$TARGET" $@
