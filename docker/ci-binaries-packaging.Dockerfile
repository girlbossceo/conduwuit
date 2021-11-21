# syntax=docker/dockerfile:1
# ---------------------------------------------------------------------------------------------------------
# This Dockerfile is intended to be built as part of Conduit's CI pipeline.
# It does not build Conduit in Docker, but just copies the matching build artifact from the build job.
# As a consequence, this is not a multiarch capable image. It always expects and packages a x86_64 binary.
#
# It is mostly based on the normal Conduit Dockerfile, but adjusted in a few places to maximise caching.
# Credit's for the original Dockerfile: Weasy666.
# ---------------------------------------------------------------------------------------------------------

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
        libgcc


ARG CREATED
ARG VERSION
ARG GIT_REF
# Labels according to https://github.com/opencontainers/image-spec/blob/master/annotations.md
# including a custom label specifying the build command
LABEL org.opencontainers.image.created=${CREATED} \
      org.opencontainers.image.authors="Conduit Contributors" \
      org.opencontainers.image.title="Conduit" \
      org.opencontainers.image.version=${VERSION} \
      org.opencontainers.image.vendor="Conduit Contributors" \
      org.opencontainers.image.description="A Matrix homeserver written in Rust" \
      org.opencontainers.image.url="https://conduit.rs/" \
      org.opencontainers.image.revision=${GIT_REF} \
      org.opencontainers.image.source="https://gitlab.com/famedly/conduit.git" \
      org.opencontainers.image.licenses="Apache-2.0" \
      org.opencontainers.image.documentation="https://gitlab.com/famedly/conduit" \
      org.opencontainers.image.ref.name=""

# Created directory for the database and media files
RUN mkdir -p /srv/conduit/.local/share/conduit

# Test if Conduit is still alive, uses the same endpoint as Element
COPY ./docker/healthcheck.sh /srv/conduit/
HEALTHCHECK --start-period=5s --interval=5s CMD ./healthcheck.sh


# Depending on the target platform (e.g. "linux/arm/v7", "linux/arm64/v8", or "linux/amd64")
# copy the matching binary into this docker image
ARG TARGETPLATFORM
COPY ./$TARGETPLATFORM /srv/conduit/conduit


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
