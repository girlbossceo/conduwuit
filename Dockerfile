# syntax=docker/dockerfile:1
FROM docker.io/rust:1.58-bullseye AS builder
WORKDIR /usr/src/conduit

# Install required packages to build Conduit and it's dependencies
RUN apt update && apt -y install libclang-11-dev

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
FROM docker.io/debian:bullseye-slim AS runner

# Standard port on which Conduit launches.
# You still need to map the port when using the docker command or docker-compose.
EXPOSE 6167

# Note from @jfowl: I would like to remove the config file in the future and just have the Docker version be configured with envs.
ENV CONDUIT_CONFIG="/srv/conduit/conduit.toml" \
    CONDUIT_PORT=6167

# Conduit needs:
#   ca-certificates: for https
#   iproute2 & wget: for the healthcheck script
RUN apt update && apt -y install \
    ca-certificates \
    iproute2 \
    wget

RUN rm -rf /var/lib/apt/lists/*

# Created directory for the database and media files
RUN mkdir -p /srv/conduit/.local/share/conduit

# Test if Conduit is still alive, uses the same endpoint as Element
COPY ./docker/healthcheck.sh /srv/conduit/healthcheck.sh
HEALTHCHECK --start-period=5s --interval=5s CMD ./healthcheck.sh

# Copy over the actual Conduit binary from the builder stage
COPY --from=builder /usr/src/conduit/target/release/conduit /srv/conduit/conduit

# Improve security: Don't run stuff as root, that does not need to run as root
# Add 'conduit' user and group (100:82). The UID:GID choice is to be compatible
# with previous, Alpine-based containers, where the user and group were both
# named 'www-data'.
RUN set -x ; \
    groupadd -r -g 82 conduit ; \
    useradd -r -M -d /srv/conduit -o -u 100 -g conduit conduit && exit 0 ; exit 1

# Change ownership of Conduit files to conduit user and group and make the healthcheck executable:
RUN chown -cR conduit:conduit /srv/conduit && \
    chmod +x /srv/conduit/healthcheck.sh

# Change user to conduit, no root permissions afterwards:
USER conduit
# Set container home directory
WORKDIR /srv/conduit

# Run Conduit and print backtraces on panics
ENV RUST_BACKTRACE=1
ENTRYPOINT [ "/srv/conduit/conduit" ]
