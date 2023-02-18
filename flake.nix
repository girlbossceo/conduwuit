{
  inputs.nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
  inputs.d2n.url = "github:nix-community/dream2nix";
  inputs.d2n.inputs.nixpkgs.follows = "nixpkgs";
  inputs.parts.url = "github:hercules-ci/flake-parts";
  inputs.rust-overlay.url = "github:oxalica/rust-overlay";
  inputs.rust-overlay.inputs.nixpkgs.follows = "nixpkgs";

  outputs = inp:
    inp.parts.lib.mkFlake {inputs = inp;} {
      systems = ["x86_64-linux"];
      imports = [inp.d2n.flakeModuleBeta];
      perSystem = {
        config,
        system,
        pkgs,
        ...
      }: let
        cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);
        pkgsWithToolchain = pkgs.appendOverlays [inp.rust-overlay.overlays.default];

        toolchains = pkgsWithToolchain.rust-bin.stable."${cargoToml.package.rust-version}";
        # toolchain to use when building conduit, includes only cargo and rustc to reduce closure size
        buildToolchain = toolchains.minimal;
        # toolchain to use in development shell
        # the "default" component set of toolchain adds rustfmt, clippy etc.
        devToolchain = toolchains.default.override {
          extensions = ["rust-src"];
        };

        # flake outputs for conduit project
        conduitOutputs = config.dream2nix.outputs.conduit;
      in {
        dream2nix.inputs.conduit = {
          source = inp.self;
          projects.conduit = {
            name = "conduit";
            subsystem = "rust";
            translator = "cargo-lock";
          };
          packageOverrides = {
            "^.*".set-toolchain.overrideRustToolchain = _: {
              cargo = buildToolchain;
              rustc = buildToolchain;
            };
          };
        };
        devShells.conduit = conduitOutputs.devShells.conduit.overrideAttrs (old: {
          # export default crate sources for rust-analyzer to read
          RUST_SRC_PATH = "${devToolchain}/lib/rustlib/src/rust/library";
          nativeBuildInputs = (old.nativeBuildInputs or []) ++ [devToolchain];
        });
        devShells.default = config.devShells.conduit;
      };
    };
}
