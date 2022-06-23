# For use in our CI only. This requires a build artifact created by a previous run pipline stage to be placed in cached_target/release/conduit
FROM valkum/docker-rust-ci:latest as builder
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


FROM valkum/docker-rust-ci:latest
WORKDIR /workdir

RUN curl -OL "https://github.com/caddyserver/caddy/releases/download/v2.2.1/caddy_2.2.1_linux_amd64.tar.gz"
RUN tar xzf caddy_2.2.1_linux_amd64.tar.gz

COPY cached_target/release/conduit /workdir/conduit
RUN chmod +x /workdir/conduit
RUN chmod +x /workdir/caddy

COPY conduit-example.toml conduit.toml

ENV SERVER_NAME=localhost
ENV CONDUIT_CONFIG=/workdir/conduit.toml

RUN sed -i "s/port = 6167/port = 8008/g" conduit.toml
RUN echo "allow_federation = true" >> conduit.toml
RUN echo "allow_encryption = true" >> conduit.toml
RUN echo "allow_registration = true" >> conduit.toml
RUN echo "log = \"info,_=off,sled=off\"" >> conduit.toml
RUN sed -i "s/address = \"127.0.0.1\"/address = \"0.0.0.0\"/g" conduit.toml

# Enabled Caddy auto cert generation for complement provided CA.
RUN echo '{"logging":{"logs":{"default":{"level":"WARN"}}}, "apps":{"http":{"https_port":8448,"servers":{"srv0":{"listen":[":8448"],"routes":[{"match":[{"host":["your.server.name"]}],"handle":[{"handler":"subroute","routes":[{"handle":[{"handler":"reverse_proxy","upstreams":[{"dial":"127.0.0.1:8008"}]}]}]}],"terminal":true}],"tls_connection_policies": [{"match": {"sni": ["your.server.name"]}}]}}},"pki": {"certificate_authorities": {"local": {"name": "Complement CA","root": {"certificate": "/ca/ca.crt","private_key": "/ca/ca.key"},"intermediate": {"certificate": "/ca/ca.crt","private_key": "/ca/ca.key"}}}},"tls":{"automation":{"policies":[{"subjects":["your.server.name"],"issuer":{"module":"internal"},"on_demand":true},{"issuer":{"module":"internal", "ca": "local"}}]}}}}' > caddy.json

EXPOSE 8008 8448

CMD ([ -z "${COMPLEMENT_CA}" ] && echo "Error: Need Complement PKI support" && true) || \
    sed -i "s/#server_name = \"your.server.name\"/server_name = \"${SERVER_NAME}\"/g" conduit.toml && \
    sed -i "s/your.server.name/${SERVER_NAME}/g" caddy.json && \
    /workdir/caddy start --config caddy.json > /dev/null && \
    /workdir/conduit
