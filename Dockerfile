# Using multistage build:
# 	https://docs.docker.com/develop/develop-images/multistage-build/
# 	https://whitfin.io/speeding-up-rust-docker-builds/


##########################  BUILD IMAGE  ##########################
# Alpine build image to build Conduit's statically compiled binary
FROM alpine:3.12 as builder

# Specifies if the local project is build or if Conduit gets build
# from the official git repository. Defaults to the git repo.
ARG LOCAL=false
# Specifies which revision/commit is build. Defaults to HEAD
ARG GIT_REF=HEAD

# Add 'edge'-repository to get Rust 1.45
RUN sed -i \
	-e 's|v3\.12|edge|' \
	/etc/apk/repositories

# Install packages needed for building all crates
RUN apk add --no-cache \
        cargo \
        openssl-dev


# Copy project files from current folder
COPY . .
# Build it from the copied local files or from the official git repository
RUN if [[ $LOCAL == "true" ]]; then \
        cargo install --path . ; \
    else \
        cargo install --git "https://git.koesters.xyz/timo/conduit.git" --rev ${GIT_REF}; \
    fi

########################## RUNTIME IMAGE ##########################
# Create new stage with a minimal image for the actual
# runtime image/container
FROM alpine:3.12

ARG CREATED
ARG VERSION
ARG GIT_REF=HEAD

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
      org.opencontainers.image.source="https://git.koesters.xyz/timo/conduit.git" \
      org.opencontainers.image.documentation.="" \
      org.opencontainers.image.licenses="AGPL-3.0" \
      org.opencontainers.image.ref.name="" \
      org.label-schema.docker.build="docker build . -t conduit_homeserver:latest --build-arg CREATED=$(date -u +'%Y-%m-%dT%H:%M:%SZ') --build-arg VERSION=$(grep -m1 -o '[0-9].[0-9].[0-9]' Cargo.toml)" \
      maintainer="Weasy666"

# Standard port on which Rocket launches
EXPOSE 8000

# Copy config files from context and the binary from
# the "builder" stage to the current stage into folder
# /srv/conduit and create data folder for database
RUN mkdir -p /srv/conduit/.local/share/conduit
COPY --from=builder /root/.cargo/bin/conduit /srv/conduit/

# Add www-data user and group with UID 82, as used by alpine
# https://git.alpinelinux.org/aports/tree/main/nginx/nginx.pre-install
RUN set -x ; \
    addgroup -Sg 82 www-data 2>/dev/null ; \
    adduser -S -D -H -h /srv/conduit -G www-data -g www-data www-data 2>/dev/null ; \
    addgroup www-data www-data 2>/dev/null && exit 0 ; exit 1

# Change ownership of Conduit files to www-data user and group
RUN chown -cR www-data:www-data /srv/conduit

# Install packages needed to run Conduit
RUN apk add --no-cache \
        ca-certificates \
        libgcc

# Create a volume for the database, to persist its contents
VOLUME ["/srv/conduit/.local/share/conduit"]

# Set user to www-data
USER www-data
# Set container home directory
WORKDIR /srv/conduit
# Run Conduit
ENTRYPOINT [ "/srv/conduit/conduit" ]
