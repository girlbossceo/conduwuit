# For use in our CI only. This requires a build artifact created by a previous run pipline stage to be placed in cached_target/release/conduit
FROM registry.gitlab.com/jfowl/conduit-containers/rust-with-tools:commit-16a08e9b as builder
#FROM rust:latest as builder

WORKDIR /workdir

ARG RUSTC_WRAPPER
ARG AWS_ACCESS_KEY_ID
ARG AWS_SECRET_ACCESS_KEY
ARG SCCACHE_BUCKET
ARG SCCACHE_ENDPOINT
ARG SCCACHE_S3_USE_SSL

COPY . .
RUN mkdir -p target/release
RUN test -e cached_target/release/conduit && cp cached_target/release/conduit target/release/conduit || cargo build --release

## Actual image
FROM debian:bullseye
WORKDIR /workdir

# Install caddy
RUN apt-get update && apt-get install -y debian-keyring debian-archive-keyring apt-transport-https curl && curl -1sLf 'https://dl.cloudsmith.io/public/caddy/testing/gpg.key' | gpg --dearmor -o /usr/share/keyrings/caddy-testing-archive-keyring.gpg && curl -1sLf 'https://dl.cloudsmith.io/public/caddy/testing/debian.deb.txt' | tee /etc/apt/sources.list.d/caddy-testing.list && apt-get update && apt-get install -y caddy

COPY conduit-example.toml conduit.toml
COPY complement/caddy.json caddy.json

ENV SERVER_NAME=localhost
ENV CONDUIT_CONFIG=/workdir/conduit.toml

RUN sed -i "s/port = 6167/port = 8008/g" conduit.toml
RUN echo "allow_federation = true" >> conduit.toml
RUN echo "allow_encryption = true" >> conduit.toml
RUN echo "allow_registration = true" >> conduit.toml
RUN echo "log = \"warn,_=off,sled=off\"" >> conduit.toml
RUN sed -i "s/address = \"127.0.0.1\"/address = \"0.0.0.0\"/g" conduit.toml

COPY --from=builder /workdir/target/release/conduit /workdir/conduit
RUN chmod +x /workdir/conduit

EXPOSE 8008 8448

CMD uname -a && \
    sed -i "s/#server_name = \"your.server.name\"/server_name = \"${SERVER_NAME}\"/g" conduit.toml && \
    sed -i "s/your.server.name/${SERVER_NAME}/g" caddy.json && \
    caddy start --config caddy.json > /dev/null && \
    /workdir/conduit
