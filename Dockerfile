# Using multistage build:
# 	https://docs.docker.com/develop/develop-images/multistage-build/
# 	https://whitfin.io/speeding-up-rust-docker-builds/


##########################  BUILD IMAGE  ##########################
# Musl build image to build Conduits statically compiled binary
FROM rustlang/rust:nightly-alpine3.12 as builder

# Don't download Rust docs
RUN rustup set profile minimal

ENV USER "conduit"
#ENV RUSTFLAGS='-C link-arg=-s'

# Install packages needed for building all crates
RUN apk add --no-cache \
        musl-dev \
        openssl-dev \
        pkgconf

# Create dummy project to fetch all dependencies.
# Rebuilds are a lot faster when there are no changes in the
# dependencies.
RUN cargo new --bin /app
WORKDIR /app

# Copy cargo files which specify needed dependencies
COPY ./Cargo.* ./

# Add musl target, as we want to run your project in
# an alpine linux image
RUN rustup target add x86_64-unknown-linux-musl

# Build dependencies and remove dummy project, except
# target folder, as it contains the dependencies
RUN cargo build --release --color=always ; \
    find . -not -path "./target*" -delete

# Now copy and build the real project with the pre-built
# dependencies.
COPY . .
RUN cargo build --release --color=always

########################## RUNTIME IMAGE ##########################
# Create new stage with a minimal image for the actual
# runtime image/container
FROM alpine:3.12

ARG BUILD_DATE
ARG VERSION
ARG GIT_REF=HEAD

# Labels inspired by this medium post:
# https://medium.com/@chamilad/lets-make-your-docker-image-better-than-90-of-existing-ones-8b1e5de950d
LABEL org.label-schema.build-date=${BUILD_DATE} \
      org.label-schema.name="Conduit" \
      org.label-schema.version=${VERSION} \
      org.label-schema.vendor="Conduit Authors" \
      org.label-schema.description="A Matrix homeserver written in Rust" \
      org.label-schema.url="https://conduit.rs/" \
      org.label-schema.vcs-ref=$GIT_REF \
      org.label-schema.vcs-url="https://git.koesters.xyz/timo/conduit.git" \
      ord.label-schema.docker.build="docker build . -t conduit:latest --build-arg BUILD_DATE=$(date -u +'%Y-%m-%dT%H:%M:%SZ') --build-arg VERSION=$(grep -m1 -o '[0-9].[0-9].[0-9]' Cargo.toml)"\
      maintainer="weasy@hotmail.de"

# Change some Rocket.rs default configs. They can then
# be changed to different values using env variables.
ENV ROCKET_CLI_COLORS="on"
#ENV ROCKET_SERVER_NAME="conduit.rs"
ENV ROCKET_ENV="production"
ENV ROCKET_ADDRESS=0.0.0.0
ENV ROCKET_PORT=14004
ENV ROCKET_LOG="normal"
ENV ROCKET_DATABASE_PATH="/data/sled"
ENV ROCKET_REGISTRATION_DISABLED="true"
#ENV ROCKET_WORKERS=10

EXPOSE 14004

# Copy config files from context and the binary from
# the "builder" stage to the current stage into folder
# /srv/conduit and create data folder for database
RUN mkdir -p /srv/conduit /data/sled

COPY --from=builder /app/target/release/conduit ./srv/conduit/

# Add www-data user and group with UID 82, as used by alpine
# https://git.alpinelinux.org/aports/tree/main/nginx/nginx.pre-install
RUN set -x ; \
    addgroup -Sg 82 www-data 2>/dev/null ; \
    adduser -S -D -H -h /srv/conduit -G www-data -g www-data www-data 2>/dev/null ; \
    addgroup www-data www-data 2>/dev/null && exit 0 ; exit 1

# Change ownership of Conduit files to www-data user and group
RUN chown -cR www-data:www-data /srv/conduit /data

VOLUME /data

RUN apk add --no-cache \
        ca-certificates

# Set user to www-data
USER www-data
WORKDIR /srv/conduit
ENTRYPOINT [ "/srv/conduit/conduit" ]
