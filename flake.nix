{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs";
    flake-utils.url = "github:numtide/flake-utils";

    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    crane = {
      url = "github:ipetkov/crane";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.flake-utils.follows = "flake-utils";
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
      pkgs = nixpkgs.legacyPackages.${system};

      # Nix-accessible `Cargo.toml`
      cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);

      # The Rust toolchain to use
      toolchain = fenix.packages.${system}.toolchainOf {
        # Use the Rust version defined in `Cargo.toml`
        channel = cargoToml.package.rust-version;

        # THE rust-version HASH
        sha256 = "sha256-8len3i8oTwJSOJZMosGGXHBL5BVuGQnWOT2St5YAUFU=";
      };

      # Point to system RocksDB
      ROCKSDB_INCLUDE_DIR = "${pkgs.rocksdb_6_23}/include";
      ROCKSDB_LIB_DIR = "${pkgs.rocksdb_6_23}/lib";

      builder =
        ((crane.mkLib pkgs).overrideToolchain toolchain.toolchain).buildPackage;
    in
    {
      packages.default = builder {
        src = ./.;

        # Use system RocksDB
        inherit ROCKSDB_INCLUDE_DIR ROCKSDB_LIB_DIR;

        nativeBuildInputs = (with pkgs.rustPlatform; [
          bindgenHook
        ]);
      };

      devShells.default = pkgs.mkShell {
        # Rust Analyzer needs to be able to find the path to default crate
        # sources, and it can read this environment variable to do so
        RUST_SRC_PATH = "${toolchain.rust-src}/lib/rustlib/src/rust/library";

        # Use system RocksDB
        inherit ROCKSDB_INCLUDE_DIR ROCKSDB_LIB_DIR;

        # Development tools
        nativeBuildInputs = (with pkgs.rustPlatform; [
          bindgenHook
        ]) ++ (with toolchain; [
          cargo
          clippy
          rust-src
          rustc
          rustfmt
        ]);
      };

      checks = {
        packagesDefault = self.packages.${system}.default;
        devShellsDefault = self.devShells.${system}.default;
      };
    });
}
