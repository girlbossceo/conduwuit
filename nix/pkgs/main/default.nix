{ inputs

# Dependencies
, craneLib
, lib
, libiconv
, pkgsBuildHost
, rocksdb
, rust
, stdenv

# Options
, features ? []
, profile ? "release"
}:

craneLib.buildPackage rec {
  src = inputs.nix-filter {
    root = inputs.self;
    include = [
      "src"
      "Cargo.toml"
      "Cargo.lock"
    ];
  };

  # This is redundant with CI
  doCheck = false;

  env =
    let
      rocksdb' = rocksdb.override {
        enableJemalloc = builtins.elem "jemalloc" features;
      };
    in
    {
      CARGO_PROFILE = profile;
      CONDUIT_VERSION_EXTRA = inputs.self.shortRev or inputs.self.dirtyShortRev;
      ROCKSDB_INCLUDE_DIR = "${rocksdb'}/include";
      ROCKSDB_LIB_DIR = "${rocksdb'}/lib";
    }
    //
    (import ./cross-compilation-env.nix {
      inherit
        lib
        pkgsBuildHost
        rust
        stdenv;
    });

  nativeBuildInputs = [
    # bindgen needs the build platform's libclang. Apparently due to "splicing
    # weirdness", pkgs.rustPlatform.bindgenHook on its own doesn't quite do the
    # right thing here.
    pkgsBuildHost.rustPlatform.bindgenHook
  ]
  ++ lib.optionals stdenv.isDarwin [ libiconv ];

  cargoExtraArgs = ""
    + lib.optionalString
      (features != [])
      "--features " + (builtins.concatStringsSep "," features);

  meta.mainProgram = (craneLib.crateNameFromCargoToml {
    cargoToml = "${inputs.self}/Cargo.toml";
  }).pname;

  passthru = {
    inherit env;
  };
}
