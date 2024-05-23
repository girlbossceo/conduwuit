# Dependencies (keep sorted)
{ craneLib
, inputs
, lib
, libiconv
, pkgsBuildHost
, rocksdb
, rust
, rust-jemalloc-sys
, stdenv

# Options (keep sorted)
, default_features ? true
, features ? []
, profile ? "release"
}:

let
featureEnabled = feature : builtins.elem feature features;

# This derivation will set the JEMALLOC_OVERRIDE variable, causing the
# tikv-jemalloc-sys crate to use the nixpkgs jemalloc instead of building it's
# own. In order for this to work, we need to set flags on the build that match
# whatever flags tikv-jemalloc-sys was going to use. These are dependent on
# which features we enable in tikv-jemalloc-sys.
rust-jemalloc-sys' = (rust-jemalloc-sys.override {
  # tikv-jemalloc-sys/unprefixed_malloc_on_supported_platforms feature
  unprefixed = true;
}).overrideAttrs (old: {
  configureFlags = old.configureFlags ++
    # tikv-jemalloc-sys/profiling feature
    lib.optional (featureEnabled "jemalloc_prof") "--enable-prof";
});

buildDepsOnlyEnv =
  let
    rocksdb' = rocksdb.override {
      jemalloc = rust-jemalloc-sys';
      enableJemalloc = featureEnabled "jemalloc";
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

    buildInputs = lib.optional (featureEnabled "jemalloc") rust-jemalloc-sys';

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
