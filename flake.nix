{
  inputs = {
    attic.url = "github:zhaofengli/attic?ref=main";
    cachix.url = "github:cachix/cachix?ref=master";
    complement = { url = "github:matrix-org/complement?ref=main"; flake = false; };
    crane = { url = "github:ipetkov/crane?ref=master"; inputs.nixpkgs.follows = "nixpkgs"; };
    fenix = { url = "github:nix-community/fenix?ref=main"; inputs.nixpkgs.follows = "nixpkgs"; };
    flake-compat = { url = "github:edolstra/flake-compat?ref=master"; flake = false; };
    flake-utils.url = "github:numtide/flake-utils?ref=main";
    nix-filter.url = "github:numtide/nix-filter?ref=main";
    nixpkgs.url = "github:NixOS/nixpkgs?ref=nixos-unstable";
    rocksdb = { url = "github:girlbossceo/rocksdb?ref=v9.4.0"; flake = false; };
    liburing = { url = "github:axboe/liburing?ref=master"; flake = false; };
  };

  outputs = inputs:
    inputs.flake-utils.lib.eachDefaultSystem (system:
    let
      pkgsHost = inputs.nixpkgs.legacyPackages.${system};
      pkgsHostStatic = pkgsHost.pkgsStatic;

      # The Rust toolchain to use
      toolchain = inputs.fenix.packages.${system}.fromToolchainFile {
        file = ./rust-toolchain.toml;

        # See also `rust-toolchain.toml`
        sha256 = "sha256-6eN/GKzjVSjEhGO9FhWObkRFaE1Jf+uqMSdQnb8lcB4=";
      };

      mkScope = pkgs: pkgs.lib.makeScope pkgs.newScope (self: {
        inherit pkgs;
        book = self.callPackage ./nix/pkgs/book {};
        complement = self.callPackage ./nix/pkgs/complement {};
        craneLib = ((inputs.crane.mkLib pkgs).overrideToolchain toolchain);
        inherit inputs;
        main = self.callPackage ./nix/pkgs/main {};
        oci-image = self.callPackage ./nix/pkgs/oci-image {};
        rocksdb = pkgs.rocksdb.overrideAttrs (old: {
          src = inputs.rocksdb;
          version = pkgs.lib.removePrefix
            "v"
            (builtins.fromJSON (builtins.readFile ./flake.lock))
              .nodes.rocksdb.original.ref;
          # we have this already at https://github.com/girlbossceo/rocksdb/commit/a935c0273e1ba44eacf88ce3685a9b9831486155
          # unsetting this so i don't have to revert it and make this nix exclusive
          patches = [];
          cmakeFlags = pkgs.lib.subtractLists
            [
              # no real reason to have snappy, no one uses this
              "-DWITH_SNAPPY=1"
              # we dont need to use ldb or sst_dump (core_tools)
              "-DWITH_CORE_TOOLS=1"
              # we dont need to build rocksdb tests
              "-DWITH_TESTS=1"
              # we use rust-rocksdb via C interface and dont need C++ RTTI
              "-DUSE_RTTI=1"
            ]
            old.cmakeFlags
            ++ [
              # we dont need to use ldb or sst_dump (core_tools)
              "-DWITH_CORE_TOOLS=0"
              # we dont need trace tools
              "-DWITH_TRACE_TOOLS=0"
              # we dont need to build rocksdb tests
              "-DWITH_TESTS=0"
              # we use rust-rocksdb via C interface and dont need C++ RTTI
              "-DUSE_RTTI=0"
            ];

          # outputs has "tools" which we dont need or use
          outputs = [ "out" ];

          # preInstall hooks has stuff for messing with ldb/sst_dump which we dont need or use
          preInstall = "";
        });
        # TODO: remove once https://github.com/NixOS/nixpkgs/pull/314945 is available
        liburing = pkgs.liburing.overrideAttrs (old: {
          # the configure script doesn't support these, and unconditionally
          # builds both static and dynamic libraries.
          configureFlags = pkgs.lib.subtractLists
            [ "--enable-static" "--disable-shared" ]
            old.configureFlags;
        });
      });

      scopeHost = mkScope pkgsHost;
      scopeHostStatic = mkScope pkgsHostStatic;

      mkDevShell = scope: scope.pkgs.mkShell {
        env = scope.main.env // {
          # Rust Analyzer needs to be able to find the path to default crate
          # sources, and it can read this environment variable to do so. The
          # `rust-src` component is required in order for this to work.
          RUST_SRC_PATH = "${toolchain}/lib/rustlib/src/rust/library";

          # Convenient way to access a pinned version of Complement's source
          # code.
          COMPLEMENT_SRC = inputs.complement.outPath;

          # Needed for Complement
          CGO_CFLAGS = "-I${scope.pkgs.olm}/include";
          CGO_LDFLAGS = "-L${scope.pkgs.olm}/lib";
        };

        # Development tools
        packages = [
          # Always use nightly rustfmt because most of its options are unstable
          #
          # This needs to come before `toolchain` in this list, otherwise
          # `$PATH` will have stable rustfmt instead.
          inputs.fenix.packages.${system}.latest.rustfmt

          toolchain
        ]
        ++ (with pkgsHost.pkgs; [
          engage
          cargo-audit

          # Needed for producing Debian packages
          cargo-deb

          # Needed for CI to check validity of produced Debian packages (dpkg-deb)
          dpkg

          # Needed for Complement
          go

          # Needed for our script for Complement
          jq

          # Needed for finding broken markdown links
          lychee

          # Needed for linting markdown files
          markdownlint-cli

          # Useful for editing the book locally
          mdbook

          # used for rust caching in CI to speed it up
          sccache

          # needed so we can get rid of gcc and other unused deps that bloat OCI images
          removeReferencesTo
        ])
        ++ scope.main.buildInputs
        ++ scope.main.propagatedBuildInputs
        ++ scope.main.nativeBuildInputs;

        meta.broken = scope.main.meta.broken;
      };
    in
    {
      packages = {
        default = scopeHost.main;
        default-debug = scopeHost.main.override {
            profile = "dev";
            # debug build users expect full logs
            disable_release_max_log_level = true;
        };
        default-test = scopeHost.main.override {
            profile = "test";
            disable_release_max_log_level = true;
        };
        all-features = scopeHost.main.override {
            all_features = true;
            disable_features = [
                # this is non-functional on nix for some reason
                "hardened_malloc"
                # dont include experimental features
                "experimental"
            ];
        };
        all-features-debug = scopeHost.main.override {
            profile = "dev";
            all_features = true;
            # debug build users expect full logs
            disable_release_max_log_level = true;
            disable_features = [
                # this is non-functional on nix for some reason
                "hardened_malloc"
                # dont include experimental features
                "experimental"
            ];
        };
        hmalloc = scopeHost.main.override { features = ["hardened_malloc"]; };

        oci-image = scopeHost.oci-image;
        oci-image-all-features = scopeHost.oci-image.override {
          main = scopeHost.main.override {
            all_features = true;
            disable_features = [
                # this is non-functional on nix for some reason
                "hardened_malloc"
                # dont include experimental features
                "experimental"
            ];
          };
        };
        oci-image-all-features-debug = scopeHost.oci-image.override {
          main = scopeHost.main.override {
            profile = "dev";
            all_features = true;
            # debug build users expect full logs
            disable_release_max_log_level = true;
            disable_features = [
                # this is non-functional on nix for some reason
                "hardened_malloc"
                # dont include experimental features
                "experimental"
            ];
          };
        };
        oci-image-hmalloc = scopeHost.oci-image.override {
          main = scopeHost.main.override {
            features = ["hardened_malloc"];
          };
        };

        book = scopeHost.book;

        complement = scopeHost.complement;
        static-complement = scopeHostStatic.complement;
      }
      //
      builtins.listToAttrs
        (builtins.concatLists
          (builtins.map
            (crossSystem:
              let
                binaryName = "static-${crossSystem}";
                pkgsCrossStatic =
                  (import inputs.nixpkgs {
                    inherit system;
                    crossSystem = {
                      config = crossSystem;
                    };
                  }).pkgsStatic;
                scopeCrossStatic = mkScope pkgsCrossStatic;
              in
              [
                # An output for a statically-linked binary
                {
                  name = binaryName;
                  value = scopeCrossStatic.main;
                }

                # An output for a statically-linked unstripped debug ("dev") binary
                {
                  name = "${binaryName}-debug";
                  value = scopeCrossStatic.main.override {
                    profile = "dev";
                    # debug build users expect full logs
                    disable_release_max_log_level = true;
                  };
                }

                # An output for a statically-linked unstripped debug binary with the
                # "test" profile (for CI usage only)
                {
                  name = "${binaryName}-test";
                  value = scopeCrossStatic.main.override {
                    profile = "test";
                    disable_release_max_log_level = true;
                  };
                }

                # An output for a statically-linked binary with `--all-features`
                {
                  name = "${binaryName}-all-features";
                  value = scopeCrossStatic.main.override {
                    all_features = true;
                    disable_features = [
                        # this is non-functional on nix for some reason
                        "hardened_malloc"
                        # dont include experimental features
                        "experimental"
                    ];
                  };
                }

                # An output for a statically-linked unstripped debug ("dev") binary with `--all-features`
                {
                  name = "${binaryName}-all-features-debug";
                  value = scopeCrossStatic.main.override {
                    profile = "dev";
                    all_features = true;
                    # debug build users expect full logs
                    disable_release_max_log_level = true;
                    disable_features = [
                        # this is non-functional on nix for some reason
                        "hardened_malloc"
                        # dont include experimental features
                        "experimental"
                    ];
                  };
                }

                # An output for a statically-linked binary with hardened_malloc
                {
                  name = "${binaryName}-hmalloc";
                  value = scopeCrossStatic.main.override {
                    features = ["hardened_malloc"];
                  };
                }

                # An output for an OCI image based on that binary
                {
                  name = "oci-image-${crossSystem}";
                  value = scopeCrossStatic.oci-image;
                }

                # An output for an OCI image based on that unstripped debug ("dev") binary
                {
                  name = "oci-image-${crossSystem}-debug";
                  value = scopeCrossStatic.oci-image.override {
                    main = scopeCrossStatic.main.override {
                        profile = "dev";
                        # debug build users expect full logs
                        disable_release_max_log_level = true;
                    };
                  };
                }

                # An output for an OCI image based on that binary with `--all-features`
                {
                  name = "oci-image-${crossSystem}-all-features";
                  value = scopeCrossStatic.oci-image.override {
                    main = scopeCrossStatic.main.override {
                      all_features = true;
                      disable_features = [
                          # this is non-functional on nix for some reason
                          "hardened_malloc"
                          # dont include experimental features
                          "experimental"
                      ];
                    };
                  };
                }

                # An output for an OCI image based on that unstripped debug ("dev") binary with `--all-features`
                {
                  name = "oci-image-${crossSystem}-all-features-debug";
                  value = scopeCrossStatic.oci-image.override {
                    main = scopeCrossStatic.main.override {
                        profile = "dev";
                        all_features = true;
                        # debug build users expect full logs
                        disable_release_max_log_level = true;
                        disable_features = [
                            # this is non-functional on nix for some reason
                            "hardened_malloc"
                            # dont include experimental features
                            "experimental"
                        ];
                    };
                  };
                }

                # An output for an OCI image based on that binary with hardened_malloc
                {
                  name = "oci-image-${crossSystem}-hmalloc";
                  value = scopeCrossStatic.oci-image.override {
                    main = scopeCrossStatic.main.override {
                      features = ["hardened_malloc"];
                    };
                  };
                }
              ]
            )
            [
              "x86_64-unknown-linux-musl"
              "aarch64-unknown-linux-musl"
            ]
          )
        );

      devShells.default = mkDevShell scopeHostStatic;
      devShells.all-features = mkDevShell
        (scopeHostStatic.overrideScope (final: prev: {
          main = prev.main.override {
            all_features = true;
            disable_features = [
                # this is non-functional on nix for some reason
                "hardened_malloc"
                # dont include experimental features
                "experimental"
            ];
        };
        }));
      devShells.no-features = mkDevShell
        (scopeHostStatic.overrideScope (final: prev: {
          main = prev.main.override { default_features = false; };
        }));
      devShells.dynamic = mkDevShell scopeHost;
    });
}
