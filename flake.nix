{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs?ref=nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";

    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    crane = {
      url = "github:ipetkov/crane";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    { self
    , nixpkgs
    , flake-utils

    , fenix
    , crane
    }: flake-utils.lib.eachDefaultSystem (system:
    let
      pkgs = import nixpkgs {
        inherit system;

        overlays = [
          (final: prev: {
            rocksdb = prev.rocksdb.overrideAttrs (old:
              let
                version = "8.10.0";
              in
              {
                inherit version;
                src = pkgs.fetchFromGitHub {
                  owner = "facebook";
                  repo = "rocksdb";
                  rev = "v${version}";
                  hash = "sha256-KGsYDBc1fz/90YYNGwlZ0LUKXYsP1zyhP29TnRQwgjQ=";
                };
              });
          })
        ];
      };

      stdenv = if pkgs.stdenv.isLinux then
        pkgs.stdenvAdapters.useMoldLinker pkgs.stdenv
      else
        pkgs.stdenv;

      # Nix-accessible `Cargo.toml`
      cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);

      # The Rust toolchain to use
      toolchain = fenix.packages.${system}.toolchainOf {
        # Use the Rust version defined in `Cargo.toml`
        channel = cargoToml.package.rust-version;

        # THE rust-version HASH
        sha256 = "sha256-PjvuouwTsYfNKW5Vi5Ye7y+lL7SsWGBxCtBOOm2z14c=";
      };

      mkToolchain = fenix.packages.${system}.combine;

      buildToolchain = mkToolchain (with toolchain; [
        cargo
        rustc
      ]);

      devToolchain = mkToolchain (with toolchain; [
        cargo
        clippy
        rust-src
        rustc

        # Always use nightly rustfmt because most of its options are unstable
        fenix.packages.${system}.latest.rustfmt
      ]);

      builder =
        ((crane.mkLib pkgs).overrideToolchain buildToolchain).buildPackage;

      nativeBuildInputs = (with pkgs.rustPlatform; [
        bindgenHook
      ]);

      env = {
        ROCKSDB_INCLUDE_DIR = "${pkgs.rocksdb}/include";
        ROCKSDB_LIB_DIR = "${pkgs.rocksdb}/lib";
      };
    in
    {
      packages.default = builder {
        src = ./.;

        inherit
          env
          nativeBuildInputs
          stdenv;
      };

      devShells.default = (pkgs.mkShell.override { inherit stdenv; }) {
        env = env // {
          # Rust Analyzer needs to be able to find the path to default crate
          # sources, and it can read this environment variable to do so. The
          # `rust-src` component is required in order for this to work.
          RUST_SRC_PATH = "${devToolchain}/lib/rustlib/src/rust/library";
        };

        # Development tools
        nativeBuildInputs = nativeBuildInputs ++ [
          devToolchain
        ] ++ (with pkgs; [
          engage
        ]);
      };
    });
}
