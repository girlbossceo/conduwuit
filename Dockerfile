# syntax=docker/dockerfile:1
FROM docker.io/rust:1.53-alpine AS builder
WORKDIR /usr/src/conduit

# Install required packages to build Conduit and it's dependencies
RUN apk add musl-dev

# == Build dependencies without our own code separately for caching ==
#
# Need a fake main.rs since Cargo refuses to build anything otherwise.
#
# See https://github.com/rust-lang/cargo/issues/2644 for a Cargo feature
# request that would allow just dependencies to be compiled, presumably
# regardless of whether source files are available.
RUN mkdir src && touch src/lib.rs && echo 'fn main() {}' > src/main.rs
COPY Cargo.toml Cargo.lock ./
RUN cargo build --release && rm -r src

# Copy over actual Conduit sources
COPY src src

# main.rs and lib.rs need their timestamp updated for this to work correctly since
# otherwise the build with the fake main.rs from above is newer than the
# source files (COPY preserves timestamps).
#
# Builds conduit and places the binary at /usr/src/conduit/target/release/conduit
RUN touch src/main.rs && touch src/lib.rs && cargo build --release




# ---------------------------------------------------------------------------------------------------------------
# Stuff below this line actually ends up in the resulting docker image
# ---------------------------------------------------------------------------------------------------------------
FROM docker.io/alpine:3.14 AS runner

# Standard port on which Conduit launches.
# You still need to map the port when using the docker command or docker-compose.
EXPOSE 6167

# Note from @jfowl: I would like to remove this in the future and just have the Docker version be configured with envs. 
ENV CONDUIT_CONFIG="/srv/conduit/conduit.toml"

# Conduit needs:
#   ca-certificates: for https
#   libgcc: Apparently this is needed, even if I (@jfowl) don't know exactly why. But whatever, it's not that big.
RUN apk add --no-cache \
        ca-certificates \
        curl \
        libgcc


# Created directory for the database and media files
RUN mkdir -p /srv/conduit/.local/share/conduit

# Test if Conduit is still alive, uses the same endpoint as Element
COPY ./docker/healthcheck.sh /srv/conduit/
HEALTHCHECK --start-period=5s --interval=5s CMD ./healthcheck.sh

# Copy over the actual Conduit binary from the builder stage
COPY --from=builder /usr/src/conduit/target/release/conduit /srv/conduit/

# Improve security: Don't run stuff as root, that does not need to run as root:
# Add www-data user and group with UID 82, as used by alpine
# https://git.alpinelinux.org/aports/tree/main/nginx/nginx.pre-install
RUN set -x ; \
    addgroup -Sg 82 www-data 2>/dev/null ; \
    adduser -S -D -H -h /srv/conduit -G www-data -g www-data www-data 2>/dev/null ; \
    addgroup www-data www-data 2>/dev/null && exit 0 ; exit 1

# Change ownership of Conduit files to www-data user and group
RUN chown -cR www-data:www-data /srv/conduit
RUN chmod +x /srv/conduit/healthcheck.sh

# Change user to www-data
USER www-data
# Set container home directory
WORKDIR /srv/conduit

# Run Conduit and print backtraces on panics
ENV RUST_BACKTRACE=1
ENTRYPOINT [ "/srv/conduit/conduit" ]