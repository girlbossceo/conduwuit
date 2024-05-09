# Dependencies (keep sorted)
{ craneLib
, inputs
, lib
, libiconv
, pkgsBuildHost
, rocksdb
, rust
, stdenv

# Options (keep sorted)
, default_features ? true
, features ? []
, profile ? "release"
}:

let
buildDepsOnlyEnv =
  let
    rocksdb' = rocksdb.override {
      enableJemalloc = builtins.elem "jemalloc" features;
    };
  in
  {
    CARGO_PROFILE = profile;
    ROCKSDB_INCLUDE_DIR = "${rocksdb'}/include";
    ROCKSDB_LIB_DIR = "${rocksdb'}/lib";
  }
  //
  (import ./cross-compilation-env.nix {
    # Keep sorted
    inherit
      lib
      pkgsBuildHost
      rust
      stdenv;
  });

buildPackageEnv = {
  CONDUWUIT_VERSION_EXTRA = inputs.self.shortRev or inputs.self.dirtyShortRev or "";
} // buildDepsOnlyEnv;

commonAttrs = {
  inherit
    (craneLib.crateNameFromCargoToml {
      cargoToml = "${inputs.self}/Cargo.toml";
    })
    pname
    version;

    src = let filter = inputs.nix-filter.lib; in filter {
      root = inputs.self;

      # Keep sorted
      include = [
        "Cargo.lock"
        "Cargo.toml"
        "deps"
        "src"
      ];
    };

    nativeBuildInputs = [
      # bindgen needs the build platform's libclang. Apparently due to "splicing
      # weirdness", pkgs.rustPlatform.bindgenHook on its own doesn't quite do the
      # right thing here.
      pkgsBuildHost.rustPlatform.bindgenHook
  ]
  ++ lib.optionals stdenv.isDarwin [
      # https://github.com/NixOS/nixpkgs/issues/206242
      libiconv

      # https://stackoverflow.com/questions/69869574/properly-adding-darwin-apple-sdk-to-a-nix-shell
      # https://discourse.nixos.org/t/compile-a-rust-binary-on-macos-dbcrossbar/8612
      pkgsBuildHost.darwin.apple_sdk.frameworks.Security
    ];
 };
in

craneLib.buildPackage ( commonAttrs // {
  cargoArtifacts = craneLib.buildDepsOnly (commonAttrs // {
    env = buildDepsOnlyEnv;
  });

  cargoExtraArgs = ""
    + lib.optionalString
      (!default_features)
      "--no-default-features "
    + lib.optionalString
      (features != [])
      "--features " + (builtins.concatStringsSep "," features);

  # This is redundant with CI
  cargoTestCommand = "";
  cargoCheckCommand = "";
  doCheck = false;

  # https://crane.dev/faq/rebuilds-bindgen.html
  NIX_OUTPATH_USED_AS_RANDOM_SEED = "aaaaaaaaaa";

  env = buildPackageEnv;

  passthru = {
    env = buildPackageEnv;
  };

  meta.mainProgram = commonAttrs.pname;
})
