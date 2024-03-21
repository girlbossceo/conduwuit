FROM rust:1.77.0

WORKDIR /workdir

RUN apt-get update && apt-get install -y --no-install-recommends \
    libclang-dev

COPY Cargo.toml Cargo.toml
COPY Cargo.lock Cargo.lock
COPY src src
RUN cargo build --release --features=axum_dual_protocol \
    && mv target/release/conduit conduit \
    && rm -rf target

COPY conduwuit-example.toml conduit.toml

ENV SERVER_NAME=localhost
ENV CONDUIT_CONFIG=/workdir/conduit.toml

RUN sed -i "s/port = 6167/port = [8448, 8008]/g" conduit.toml
RUN sed -i "s/allow_registration = false/allow_registration = true/g" conduit.toml
RUN sed -i "s/registration_token/#registration_token/g" conduit.toml
RUN sed -i "s/allow_guest_registration = false/allow_guest_registration = true/g" conduit.toml
RUN sed -i "s/allow_public_room_directory_over_federation = false/allow_public_room_directory_over_federation = true/g" conduit.toml
RUN sed -i "s/allow_public_room_directory_without_auth = false/allow_public_room_directory_without_auth = true/g" conduit.toml
RUN sed -i "s/allow_device_name_federation = false/allow_device_name_federation = true/g" conduit.toml
RUN sed -i "/\"127.0.0.0/d" conduit.toml
RUN sed -i "/\"10.0.0.0/d" conduit.toml
RUN sed -i "/\"172.16.0.0/d" conduit.toml
RUN sed -i "/\"::1/d" conduit.toml
RUN sed -i "s/#log = \"warn\"/log = \"debug\"/g" conduit.toml
RUN sed -i 's/#\strusted_servers\s=\s\["matrix.org"\]/trusted_servers = []/g' conduit.toml
RUN sed -i 's/# `yes_i_am_very_very_sure_i_want_an_open_registration_server_prone_to_abuse` to/yes_i_am_very_very_sure_i_want_an_open_registration_server_prone_to_abuse = true/g' conduit.toml
RUN sed -i "s/allow_outgoing_presence = false/allow_outgoing_presence = true/g" conduit.toml
RUN sed -i "s/allow_incoming_presence = false/allow_incoming_presence = true/g" conduit.toml
RUN sed -i "s/allow_local_presence = false/allow_local_presence = true/g" conduit.toml
RUN sed -i "s/address = \"127.0.0.1\"/address = \"0.0.0.0\"/g" conduit.toml

# https://stackoverflow.com/questions/76049656/unexpected-notvalidforname-with-rusts-tonic-with-tls
RUN echo "authorityKeyIdentifier=keyid,issuer" >> extensions.ext
RUN echo "basicConstraints=CA:FALSE" >> extensions.ext
RUN echo 'subjectAltName = @alt_names' >> extensions.ext
RUN echo '[alt_names]' >> extensions.ext
RUN echo "DNS.1 = servername" >> extensions.ext
RUN echo "IP.1 = ipaddress" >> extensions.ext


EXPOSE 8008 8448

CMD uname -a && \
    cp -f -v /complement/ca/ca.crt /usr/local/share/ca-certificates/complement.crt && \
    update-ca-certificates && \
    sed -i "s/servername/${SERVER_NAME}/g" extensions.ext && \
    sed -i "s/ipaddress/`hostname -i`/g" extensions.ext && \
    openssl req -newkey rsa:2048 -noenc -subj "/C=US/ST=CA/O=MyOrg, Inc./CN=$SERVER_NAME" -keyout $SERVER_NAME.key -out $SERVER_NAME.csr && \
    openssl x509 -signkey $SERVER_NAME.key -in $SERVER_NAME.csr -req -days 2 -out $SERVER_NAME.crt && \
    openssl x509 -req -CA /complement/ca/ca.crt -CAkey /complement/ca/ca.key -in $SERVER_NAME.csr -out $SERVER_NAME.crt -days 2 -CAcreateserial -extfile extensions.ext && \
    sed -i "s/#server_name = \"your.server.name\"/server_name = \"${SERVER_NAME}\"/g" conduit.toml && \
    sed -i 's/#\s\[global.tls\]/\[global.tls\]/g' conduit.toml && \
    sed -i "s/# certs = \"\/path\/to\/my\/certificate.crt\"/certs = \"${SERVER_NAME}.crt\"/g" conduit.toml && \
    sed -i "s/# key = \"\/path\/to\/my\/private_key.key\"/key = \"${SERVER_NAME}.key\"/g" conduit.toml && \
    sed -i "s/#dual_protocol = false/dual_protocol = true/g" conduit.toml && \
    /workdir/conduit

