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
  CONDUIT_VERSION_EXTRA = inputs.self.shortRev or inputs.self.dirtyShortRev;
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
        "hot_lib"
        "src"
      ];
    };

    nativeBuildInputs = [
      # bindgen needs the build platform's libclang. Apparently due to "splicing
      # weirdness", pkgs.rustPlatform.bindgenHook on its own doesn't quite do the
      # right thing here.
      pkgsBuildHost.rustPlatform.bindgenHook
  ]
  ++ lib.optionals stdenv.isDarwin [ libiconv ];
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
  doCheck = false;

  env = buildPackageEnv;

  passthru = {
    env = buildPackageEnv;
  };

  meta.mainProgram = commonAttrs.pname;
})
