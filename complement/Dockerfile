FROM rust:1.75.0

WORKDIR /workdir

RUN apt-get update && apt-get install -y --no-install-recommends \
    libclang-dev

COPY Cargo.toml Cargo.toml
COPY Cargo.lock Cargo.lock
COPY src src
RUN cargo build --release \
    && mv target/release/conduit conduit \
    && rm -rf target

# Install caddy
RUN apt-get update \
    && apt-get install -y \
    debian-keyring \
    debian-archive-keyring \
    apt-transport-https \
    curl \
    && curl -1sLf 'https://dl.cloudsmith.io/public/caddy/testing/gpg.key' \
    | gpg --dearmor -o /usr/share/keyrings/caddy-testing-archive-keyring.gpg \
    && curl -1sLf 'https://dl.cloudsmith.io/public/caddy/testing/debian.deb.txt' \
    | tee /etc/apt/sources.list.d/caddy-testing.list \
    && apt-get update \
    && apt-get install -y caddy

COPY conduwuit-example.toml conduit.toml
COPY complement/caddy.json caddy.json

ENV SERVER_NAME=localhost
ENV CONDUIT_CONFIG=/workdir/conduit.toml

RUN sed -i "s/port = 6167/port = 8008/g" conduit.toml
RUN sed -i "s/allow_registration = false/allow_registration = true/g" conduit.toml
RUN sed -i "s/registration_token/#registration_token/g" conduit.toml
RUN sed -i "s/allow_guest_registration = false/allow_guest_registration = true/g" conduit.toml
RUN sed -i "s/allow_public_room_directory_over_federation = false/allow_public_room_directory_over_federation = true/g" conduit.toml
RUN sed -i "s/allow_public_room_directory_without_auth = false/allow_public_room_directory_without_auth = true/g" conduit.toml
RUN sed -i "s/allow_device_name_federation = false/allow_device_name_federation = true/g" conduit.toml
RUN sed -i "/\"127.0.0.0/d" conduit.toml
RUN sed -i "/\"10.0.0.0/d" conduit.toml
RUN sed -i "/\"::1/d" conduit.toml
RUN echo "log = \"warn\"" >> conduit.toml
RUN echo 'yes_i_am_very_very_sure_i_want_an_open_registration_server_prone_to_abuse = true' >> conduit.toml
RUN echo 'allow_outgoing_presence = true' >> conduit.toml
RUN echo 'allow_incoming_presence = true' >> conduit.toml
RUN echo 'allow_local_presence = true' >> conduit.toml
RUN sed -i "s/address = \"127.0.0.1\"/address = \"0.0.0.0\"/g" conduit.toml

EXPOSE 8008 8448

CMD uname -a && \
    sed -i "s/#server_name = \"your.server.name\"/server_name = \"${SERVER_NAME}\"/g" conduit.toml && \
    sed -i "s/your.server.name/${SERVER_NAME}/g" caddy.json && \
    caddy start --config caddy.json > /dev/null && \
    /workdir/conduit

