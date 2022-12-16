{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs";
    flake-utils.url = "github:numtide/flake-utils";

    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    naersk = {
      url = "github:nix-community/naersk";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    { self
    , nixpkgs
    , flake-utils

    , fenix
    , naersk
    }: flake-utils.lib.eachDefaultSystem (system:
    let
      pkgs = nixpkgs.legacyPackages.${system};

      # Nix-accessible `Cargo.toml`
      cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);

      # The Rust toolchain to use
      toolchain = fenix.packages.${system}.toolchainOf {
        # Use the Rust version defined in `Cargo.toml`
        channel = cargoToml.package.rust-version;

        # This will need to be updated when `package.rust-version` is changed in
        # `Cargo.toml`
        sha256 = "sha256-8len3i8oTwJSOJZMosGGXHBL5BVuGQnWOT2St5YAUFU=";
      };

      builder = (pkgs.callPackage naersk {
        inherit (toolchain) rustc cargo;
      }).buildPackage;
    in
    {
      packages.default = builder {
        src = ./.;

        nativeBuildInputs = (with pkgs.rustPlatform; [
          bindgenHook
        ]);
      };

      devShells.default = pkgs.mkShell {
        # Rust Analyzer needs to be able to find the path to default crate
        # sources, and it can read this environment variable to do so
        RUST_SRC_PATH = "${toolchain.rust-src}/lib/rustlib/src/rust/library";

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
