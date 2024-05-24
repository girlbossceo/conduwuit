{
  inputs = {
    attic.url = "github:zhaofengli/attic?ref=main";
    complement = { url = "github:matrix-org/complement?ref=main"; flake = false; };
    crane = { url = "github:ipetkov/crane?ref=master"; inputs.nixpkgs.follows = "nixpkgs"; };
    fenix = { url = "github:nix-community/fenix?ref=main"; inputs.nixpkgs.follows = "nixpkgs"; };
    flake-compat = { url = "github:edolstra/flake-compat?ref=master"; flake = false; };
    flake-utils.url = "github:numtide/flake-utils?ref=main";
    nix-filter.url = "github:numtide/nix-filter?ref=main";
    nixpkgs.url = "github:NixOS/nixpkgs?ref=nixos-unstable";
    # https://github.com/girlbossceo/rocksdb/commit/db6df0b185774778457dabfcbd822cb81760cade
    rocksdb = { url = "github:girlbossceo/rocksdb?ref=v9.1.1"; flake = false; };
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
        sha256 = "sha256-+syqAd2kX8KVa8/U2gz3blIQTTsYYt3U63xBWaGOSc8";
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

          # Needed for Complement
          go
          olm

          # Needed for our script for Complement
          jq

          # Needed for finding broken markdown links
          lychee

          # Useful for editing the book locally
          mdbook
        ])
        ++ scope.main.propagatedBuildInputs
        ++ scope.main.nativeBuildInputs;

        meta.broken = scope.main.meta.broken;
      };
    in
    {
      packages = {
        default = scopeHost.main;
        jemalloc = scopeHost.main.override { features = ["jemalloc"]; };
        hmalloc = scopeHost.main.override { features = ["hardened_malloc"]; };

        oci-image = scopeHost.oci-image;
        oci-image-jemalloc = scopeHost.oci-image.override {
          main = scopeHost.main.override {
            features = ["jemalloc"];
          };
        };
        oci-image-hmalloc = scopeHost.oci-image.override {
          main = scopeHost.main.override {
            features = ["hardened_malloc"];
          };
        };

        book = scopeHost.book;

        complement = scopeHost.complement;
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

                # An output for a statically-linked binary with jemalloc
                {
                  name = "${binaryName}-jemalloc";
                  value = scopeCrossStatic.main.override {
                    features = ["jemalloc"];
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

                # An output for an OCI image based on that binary with jemalloc
                {
                  name = "oci-image-${crossSystem}-jemalloc";
                  value = scopeCrossStatic.oci-image.override {
                    main = scopeCrossStatic.main.override {
                      features = ["jemalloc"];
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
          main = prev.main.override { all_features = true; };
        }));
      devShells.no-features = mkDevShell
        (scopeHostStatic.overrideScope (final: prev: {
          main = prev.main.override { default_features = false; };
        }));
      devShells.dynamic = mkDevShell scopeHost;
    });
}
