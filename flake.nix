{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs?ref=nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    nix-filter.url = "github:numtide/nix-filter";
    flake-compat = {
      url = "github:edolstra/flake-compat";
      flake = false;
    };

    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    crane = {
      # Pin latest crane that's not affected by the following bugs:
      #
      # * <https://github.com/ipetkov/crane/issues/527#issuecomment-1978079140>
      # * <https://github.com/toml-rs/toml/issues/691>
      # * <https://github.com/toml-rs/toml/issues/267>
      url = "github:ipetkov/crane?rev=2c653e4478476a52c6aa3ac0495e4dea7449ea0e";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    attic.url = "github:zhaofengli/attic?ref=main";
  };

  outputs =
    { self
    , nixpkgs
    , flake-utils
    , nix-filter

    , fenix
    , crane
    , ...
    }: flake-utils.lib.eachDefaultSystem (system:
    let
      pkgsHost = nixpkgs.legacyPackages.${system};
      allocator = null;

      rocksdb' = pkgs:
        let
          version = "9.1.0";
        in
        (pkgs.rocksdb.overrideAttrs (old: {
          inherit version;
          src = pkgs.fetchFromGitHub {
            owner = "facebook";
            repo = "rocksdb";
            rev = "bcf88d48ce8aa8b536aee4dd305533b3b83cf435";
            hash = "sha256-vRPyrXkXVVhP56n5FVYef8zbIsnnanQSpElmQLZ7mh8";
          };
        }));

      # Nix-accessible `Cargo.toml`
      cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);

      # The Rust toolchain to use
      toolchain = fenix.packages.${system}.fromToolchainFile {
        file = ./rust-toolchain.toml;

        # See also `rust-toolchain.toml`
        sha256 = "sha256-SXRtAuO4IqNOQq+nLbrsDFbVk+3aVA8NNpSZsKlVH/8=";
      };

      builder = pkgs:
        ((crane.mkLib pkgs).overrideToolchain toolchain).buildPackage;

      nativeBuildInputs = pkgs: [
        # bindgen needs the build platform's libclang. Apparently due to
        # "splicing weirdness", pkgs.rustPlatform.bindgenHook on its own doesn't
        # quite do the right thing here.
        pkgs.pkgsBuildHost.rustPlatform.bindgenHook
      ];

      env = pkgs: {
        CONDUIT_VERSION_EXTRA = self.shortRev or self.dirtyShortRev;
        ROCKSDB_INCLUDE_DIR = "${rocksdb' pkgs}/include";
        ROCKSDB_LIB_DIR = "${rocksdb' pkgs}/lib";
      }
      // pkgs.lib.optionalAttrs pkgs.stdenv.hostPlatform.isStatic {
        ROCKSDB_STATIC = "";
      }
      // {
        CARGO_BUILD_RUSTFLAGS = let inherit (pkgs) lib stdenv; in
          lib.concatStringsSep " " ([ ]
            ++ lib.optionals
            # This disables PIE for static builds, which isn't great in terms
            # of security. Unfortunately, my hand is forced because nixpkgs'
            # `libstdc++.a` is built without `-fPIE`, which precludes us from
            # leaving PIE enabled.
            stdenv.hostPlatform.isStatic
            [ "-C" "relocation-model=static" ]
            ++ lib.optionals
            (stdenv.buildPlatform.config != stdenv.hostPlatform.config)
            [ "-l" "c" ]
            ++ lib.optionals
            # This check has to match the one [here][0]. We only need to set
            # these flags when using a different linker. Don't ask me why,
            # though, because I don't know. All I know is it breaks otherwise.
            #
            # [0]: https://github.com/NixOS/nixpkgs/blob/5cdb38bb16c6d0a38779db14fcc766bc1b2394d6/pkgs/build-support/rust/lib/default.nix#L37-L40
            (
              # Nixpkgs doesn't check for x86_64 here but we do, because I
              # observed a failure building statically for x86_64 without
              # including it here. Linkers are weird.
              (stdenv.hostPlatform.isAarch64 || stdenv.hostPlatform.isx86_64)
                && stdenv.hostPlatform.isStatic
                && !stdenv.isDarwin
                && !stdenv.cc.bintools.isLLVM
            )
            [
              "-l"
              "stdc++"
              "-L"
              "${stdenv.cc.cc.lib}/${stdenv.hostPlatform.config}/lib"
            ]
          );
      }

      # What follows is stolen from [here][0]. Its purpose is to properly
      # configure compilers and linkers for various stages of the build, and
      # even covers the case of build scripts that need native code compiled and
      # run on the build platform (I think).
      #
      # [0]: https://github.com/NixOS/nixpkgs/blob/5cdb38bb16c6d0a38779db14fcc766bc1b2394d6/pkgs/build-support/rust/lib/default.nix#L57-L80
      // (
        let
          inherit (pkgs.rust.lib) envVars;
        in
        pkgs.lib.optionalAttrs
          (pkgs.stdenv.targetPlatform.rust.rustcTarget
            != pkgs.stdenv.hostPlatform.rust.rustcTarget)
          (
            let
              inherit (pkgs.stdenv.targetPlatform.rust) cargoEnvVarTarget;
            in
            {
              "CC_${cargoEnvVarTarget}" = envVars.ccForTarget;
              "CXX_${cargoEnvVarTarget}" = envVars.cxxForTarget;
              "CARGO_TARGET_${cargoEnvVarTarget}_LINKER" =
                envVars.linkerForTarget;
            }
          )
        // (
          let
            inherit (pkgs.stdenv.hostPlatform.rust) cargoEnvVarTarget rustcTarget;
          in
          {
            "CC_${cargoEnvVarTarget}" = envVars.ccForHost;
            "CXX_${cargoEnvVarTarget}" = envVars.cxxForHost;
            "CARGO_TARGET_${cargoEnvVarTarget}_LINKER" = envVars.linkerForHost;
            CARGO_BUILD_TARGET = rustcTarget;
          }
        )
        // (
          let
            inherit (pkgs.stdenv.buildPlatform.rust) cargoEnvVarTarget;
          in
          {
            "CC_${cargoEnvVarTarget}" = envVars.ccForBuild;
            "CXX_${cargoEnvVarTarget}" = envVars.cxxForBuild;
            "CARGO_TARGET_${cargoEnvVarTarget}_LINKER" = envVars.linkerForBuild;
            HOST_CC = "${pkgs.pkgsBuildHost.stdenv.cc}/bin/cc";
            HOST_CXX = "${pkgs.pkgsBuildHost.stdenv.cc}/bin/c++";
          }
        )
      );

      mkPackage = pkgs: allocator: cargoArgs: profile: builder pkgs {
        src = nix-filter {
          root = ./.;
          include = [
            "src"
            "Cargo.toml"
            "Cargo.lock"
          ];
        };

        buildFeatures = [ ]
          ++ (if allocator == "jemalloc" then [ "jemalloc" ] else [ ])
          ++ (if allocator == "hmalloc" then [ "hardened_malloc" ] else [ ])
        ;

        rocksdb' = (if allocator == "jemalloc" then (pkgs.rocksdb.override { enableJemalloc = true; }) else (rocksdb' pkgs));

        # This is redundant with CI
        doCheck = false;

        env = env pkgs;
        nativeBuildInputs = nativeBuildInputs pkgs;

        cargoExtraArgs = cargoArgs;

        meta.mainProgram = cargoToml.package.name;

        CARGO_PROFILE = profile;
      };

      mkOciImage = pkgs: package:
        pkgs.dockerTools.buildLayeredImage {
          name = package.pname;
          tag = "main";
          # Debian makes builds reproducible through using the HEAD commit's date
          created = "@${toString self.lastModified}";
          contents = [
            pkgs.dockerTools.caCertificates
          ];
          config = {
            # Use the `tini` init system so that signals (e.g. ctrl+c/SIGINT)
            # are handled as expected
            Entrypoint = if !pkgs.stdenv.isDarwin then [
              "${pkgs.lib.getExe' pkgs.tini "tini"}"
              "--"
            ] else [];
            Cmd = [
              "${pkgs.lib.getExe package}"
            ];
          };
        };

        createComplementRuntime = pkgs: image: let
          complement = pkgs.fetchFromGitHub {
            owner = "matrix-org";
            repo = "complement";
            rev = "d73c81a091604b0fc5b6b0617dcac58c25763f57";
            hash = "sha256-hom/Lt0gZzLWqFhUJG0X2i88CAMIILInO5w0tPj6G3s";
          };

          script = pkgs.writeShellScriptBin "run.sh"
            ''
            export PATH=${pkgs.lib.makeBinPath [ pkgs.olm pkgs.gcc ]}
            ${pkgs.lib.getExe pkgs.docker} load < ${image}
            set +o pipefail
            /usr/bin/env -C "${complement}" COMPLEMENT_BASE_IMAGE="complement-conduit:dev" ${pkgs.lib.getExe pkgs.go} test -json ${complement}/tests | ${pkgs.toybox}/bin/tee $1
            set -o pipefail

            # Post-process the results into an easy-to-compare format
            ${pkgs.coreutils}/bin/cat "$1" | ${pkgs.lib.getExe pkgs.jq} -c '
            select(
              (.Action == "pass" or .Action == "fail" or .Action == "skip")
              and .Test != null
            ) | {Action: .Action, Test: .Test}
            ' | ${pkgs.coreutils}/bin/sort > "$2"
            '';

        in script;

        createComplementImage = pkgs: let

          conduwuit = mkPackage pkgs "jemalloc" "--features=axum_dual_protocol" "dev";

          in pkgs.dockerTools.buildImage {
            name = "complement-conduit";
            tag = "dev";

            copyToRoot = pkgs.stdenv.mkDerivation {

              name = "complement_data";
              src = nix-filter {
                root = ./.;
                include = [
                  "tests/complement/conduwuit-complement.toml"
                  "tests/complement/v3.ext"
                ];
              };
              phases = [ "unpackPhase" "installPhase" ];
              installPhase = ''
                mkdir -p $out/conduwuit/data
                cp $src/tests/complement/conduwuit-complement.toml $out/conduwuit/conduit.toml
                cp $src/tests/complement/v3.ext $out/v3.ext
              '';

            };

            config = {

              Cmd = [
                  "${pkgs.bash}/bin/sh"
                  "-c"
                  ''
                  echo "Starting server as $SERVER_NAME" &&
                  export CONDUIT_SERVER_NAME=$SERVER_NAME CONDUIT_WELL_KNOWN_SERVER="$SERVER_NAME:8448" CONDUIT_WELL_KNOWN_SERVER="$SERVER_NAME:8008" &&
                  ${pkgs.lib.getExe pkgs.openssl} genrsa -out /conduwuit/private_key.key 2048 &&
                  ${pkgs.lib.getExe pkgs.openssl} req -new -sha256 -key /conduwuit/private_key.key -subj "/C=US/ST=CA/O=MyOrg, Inc./CN=$SERVER_NAME" -out /conduwuit/signing_request.csr &&
                  echo "DNS.1 = $SERVER_NAME" >> /v3.ext &&
                  echo "IP.1 = $(${pkgs.lib.getExe pkgs.gawk} 'END{print $1}' /etc/hosts)" >> /v3.ext &&
                  ${pkgs.lib.getExe pkgs.openssl} x509 -req -extfile /v3.ext -in /conduwuit/signing_request.csr -CA /complement/ca/ca.crt -CAkey /complement/ca/ca.key -CAcreateserial -out /conduwuit/certificate.crt -days 1 -sha256 &&
                  ${pkgs.lib.getExe conduwuit}
                  ''
              ];

              Entrypoint = if !pkgs.stdenv.isDarwin then [
                "${pkgs.lib.getExe' pkgs.tini "tini"}"
                "--"
              ] else [];

              Env = [
                "SSL_CERT_FILE=/complement/ca/ca.crt"
                "SERVER_NAME=localhost"
                "CONDUIT_CONFIG=/conduwuit/conduit.toml"
              ];

              ExposedPorts = {
                "8008/tcp" = {};
                "8448/tcp" = {};
              };

            };
        };
    in
    {
      packages = {
        default = mkPackage pkgsHost null "" "release";
        jemalloc = mkPackage pkgsHost "jemalloc" "" "release";
        hmalloc = mkPackage pkgsHost "hmalloc" "" "release";
        oci-image = mkOciImage pkgsHost self.packages.${system}.default;
        oci-image-jemalloc = mkOciImage pkgsHost self.packages.${system}.jemalloc;
        oci-image-hmalloc = mkOciImage pkgsHost self.packages.${system}.hmalloc;

        book =
          let
            package = self.packages.${system}.default;
          in
          pkgsHost.stdenv.mkDerivation {
            pname = "${package.pname}-book";
            version = package.version;

            src = nix-filter {
              root = ./.;
              include = [
                "book.toml"
                "conduwuit-example.toml"
                "README.md"
                "debian/README.md"
                "docs"
              ];
            };

            nativeBuildInputs = (with pkgsHost; [
              mdbook
            ]);

            buildPhase = ''
              mdbook build
              mv public $out
            '';
          };
          complement-image = createComplementImage pkgsHost;
          complement-runtime = createComplementRuntime pkgsHost self.outputs.packages.${system}.complement-image;
      }
      //
      builtins.listToAttrs
        (builtins.concatLists
          (builtins.map
            (crossSystem:
              let
                binaryName = "static-${crossSystem}";
                pkgsCrossStatic =
                  (import nixpkgs {
                    inherit system;
                    crossSystem = {
                      config = crossSystem;
                    };
                  }).pkgsStatic;
              in
              [
                # An output for a statically-linked binary
                {
                  name = binaryName;
                  value = mkPackage pkgsCrossStatic null "" "release";
                }

                # An output for a statically-linked binary with jemalloc
                {
                  name = "${binaryName}-jemalloc";
                  value = mkPackage pkgsCrossStatic "jemalloc" "" "release";
                }

                # An output for a statically-linked binary with hardened_malloc
                {
                  name = "${binaryName}-hmalloc";
                  value = mkPackage pkgsCrossStatic "hmalloc" "" "release";
                }

                # An output for an OCI image based on that binary
                {
                  name = "oci-image-${crossSystem}";
                  value = mkOciImage
                    pkgsCrossStatic
                    self.packages.${system}.${binaryName};
                }

                # An output for an OCI image based on that binary with jemalloc
                {
                  name = "oci-image-${crossSystem}-jemalloc";
                  value = mkOciImage
                    pkgsCrossStatic
                    self.packages.${system}.${binaryName};
                }

                # An output for an OCI image based on that binary with hardened_malloc
                {
                  name = "oci-image-${crossSystem}-hmalloc";
                  value = mkOciImage
                    pkgsCrossStatic
                    self.packages.${system}.${binaryName};
                }
              ]
            )
            [
              "x86_64-unknown-linux-musl"
              "aarch64-unknown-linux-musl"
            ]
          )
        );

      devShells.default = pkgsHost.mkShell {
        env = env pkgsHost // {
          # Rust Analyzer needs to be able to find the path to default crate
          # sources, and it can read this environment variable to do so. The
          # `rust-src` component is required in order for this to work.
          RUST_SRC_PATH = "${toolchain}/lib/rustlib/src/rust/library";
        };

        # Development tools
        nativeBuildInputs = nativeBuildInputs pkgsHost ++ [
          # Always use nightly rustfmt because most of its options are unstable
          #
          # This needs to come before `toolchain` in this list, otherwise
          # `$PATH` will have stable rustfmt instead.
          fenix.packages.${system}.latest.rustfmt

          toolchain
        ] ++ (with pkgsHost; [
          engage

          # Needed for producing Debian packages
          cargo-deb

          # Needed for Complement
          go
          olm

          # Needed for our script for Complement
          jq

          # Needed for finding broken markdown links
          lychee

          # Useful for editing the book locally
          mdbook
        ]);
      };
    });
}
