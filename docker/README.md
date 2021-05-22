# Deploy using Docker

> **Note:** To run and use Conduit you should probably use it with a Domain or Subdomain behind a reverse proxy (like Nginx, Traefik, Apache, ...) with a Lets Encrypt certificate.


## Docker

### Build & Dockerfile

The Dockerfile provided by Conduit has two stages, each of which creates an image.
1. **Builder:** Builds the binary from local context or by cloning a git revision from the official repository.
2. **Runtime:** Copies the built binary from **Builder** and sets up the runtime environment, like creating a volume to persist the database and applying the correct permissions.

The Dockerfile includes a few build arguments that should be supplied when building it.

``` Dockerfile
ARG LOCAL=false
ARG CREATED
ARG VERSION
ARG GIT_REF=origin/master
```

- **CREATED:** Date and time as string (date-time as defined by RFC 3339). Will be used to create the Open Container Initiative compliant label `org.opencontainers.image.created`. Supply by it like this `$(date -u +'%Y-%m-%dT%H:%M:%SZ')`
- **VERSION:** The SemVer version of Conduit, which is in the image. Will be used to create the Open Container Initiative compliant label `org.opencontainers.image.version`. If you have a `Cargo.toml` in your build context, you can get it with `$(grep -m1 -o '[0-9].[0-9].[0-9]' Cargo.toml)`
- **LOCAL:** *(Optional)* A boolean value, specifies if the local build context should be used, or if the official repository will be cloned. If not supplied with the build command, it will default to `false`.
- **GIT_REF:** *(Optional)* A git ref, like `HEAD` or a commit ID. The supplied ref will be used to create the Open Container Initiative compliant label `org.opencontainers.image.revision` and will be the ref that is cloned from the repository when not building from the local context. If not supplied with the build command, it will default to `origin/master`.

To build the image you can use the following command

``` bash
docker build . -t matrixconduit/matrix-conduit:latest --build-arg CREATED=$(date -u +'%Y-%m-%dT%H:%M:%SZ') --build-arg VERSION=$(grep -m1 -o '[0-9].[0-9].[0-9]' Cargo.toml)
```

which also will tag the resulting image as `matrixconduit/matrix-conduit:latest`.
**Note:** it ommits the two optional `build-arg`s.


### Run

After building the image you can simply run it with

``` bash
docker run -d -p 8448:8000 -v ~/conduit.toml:/srv/conduit/conduit.toml -v db:/srv/conduit/.local/share/conduit matrixconduit/matrix-conduit:latest
```

For detached mode, you also need to use the `-d` flag. You also need to supply a `conduit.toml` config file, you can find an example [here](../conduit-example.toml).
You can pass in different env vars to change config values on the fly. You can even configure Conduit completely by using env vars, but for that you need
too pass `-e CONDUIT_CONFIG=""` into your container. For an overview of possible values, please take a look at the `docker-compose.yml` file.
If you just want to test Conduit for a short time, you can use the `--rm` flag, which will clean up everything related to your container after you stop it.


## Docker-compose

If the docker command is not for you or your setup, you can also use one of the provided `docker-compose` files. Depending on your proxy setup, use the [`docker-compose.traefik.yml`](docker-compose.traefik.yml) including [`docker-compose.override.traefik.yml`](docker-compose.override.traefik.yml) or the normal [`docker-compose.yml`](../docker-compose.yml) for every other reverse proxy.


### Build

To build the Conduit image with docker-compose, you first need to open and modify the `docker-compose.yml` file. There you need to comment the `image:` option and uncomment the `build:` option. Then call docker-compose with:

``` bash
CREATED=$(date -u +'%Y-%m-%dT%H:%M:%SZ') VERSION=$(grep -m1 -o '[0-9].[0-9].[0-9]' Cargo.toml) docker-compose up
```

This will also start the container right afterwards, so if want it to run in detached mode, you also should use the `-d` flag. For possible `build-args`, please take a look at the above `Build & Dockerfile` section.


### Run

If you already have built the image, you can just start the container and everything else in the compose file in detached mode with:

``` bash
docker-compose up -d
```
