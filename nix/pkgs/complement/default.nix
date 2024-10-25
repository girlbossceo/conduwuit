# Dependencies
{ bashInteractive
, buildEnv
, coreutils
, dockerTools
, gawk
, lib
, main
, openssl
, stdenv
, tini
, writeShellScriptBin
}:

let
  main' = main.override {
    profile = "test";
    all_features = true;
    disable_release_max_log_level = true;
    disable_features = [
        # no reason to use jemalloc for complement, just has compatibility/build issues
        "jemalloc"
        # console/CLI stuff isn't used or relevant for complement
        "console"
        "tokio_console"
        # sentry telemetry isn't useful for complement, disabled by default anyways
        "sentry_telemetry"
        # the containers don't use or need systemd signal support
        "systemd"
        # this is non-functional on nix for some reason
        "hardened_malloc"
        # dont include experimental features
        "experimental"
    ];
  };

  start = writeShellScriptBin "start" ''
    set -euxo pipefail

    ${lib.getExe openssl} genrsa -out private_key.key 2048
    ${lib.getExe openssl} req \
      -new \
      -sha256 \
      -key private_key.key \
      -subj "/C=US/ST=CA/O=MyOrg, Inc./CN=$SERVER_NAME" \
      -out signing_request.csr
    cp ${./v3.ext} v3.ext
    echo "DNS.1 = $SERVER_NAME" >> v3.ext
    echo "IP.1 = $(${lib.getExe gawk} 'END{print $1}' /etc/hosts)" \
      >> v3.ext
    ${lib.getExe openssl} x509 \
      -req \
      -extfile v3.ext \
      -in signing_request.csr \
      -CA /complement/ca/ca.crt \
      -CAkey /complement/ca/ca.key \
      -CAcreateserial \
      -out certificate.crt \
      -days 1 \
      -sha256

    ${lib.getExe' coreutils "env"} \
      CONDUWUIT_SERVER_NAME="$SERVER_NAME" \
      ${lib.getExe main'}
  '';
in

dockerTools.buildImage {
  name = "complement-conduwuit";
  tag = "main";

  copyToRoot = buildEnv {
    name = "root";
    pathsToLink = [
      "/bin"
    ];
    paths = [
      bashInteractive
      coreutils
      main'
      start
    ];
  };

  config = {
    Cmd = [
      "${lib.getExe start}"
    ];

    Entrypoint = if !stdenv.hostPlatform.isDarwin
      # Use the `tini` init system so that signals (e.g. ctrl+c/SIGINT)
      # are handled as expected
      then [ "${lib.getExe' tini "tini"}" "--" ]
      else [];

    Env = [
      "SSL_CERT_FILE=/complement/ca/ca.crt"
      "CONDUWUIT_CONFIG=${./config.toml}"
    ];

    ExposedPorts = {
      "8008/tcp" = {};
      "8448/tcp" = {};
    };
  };
}
