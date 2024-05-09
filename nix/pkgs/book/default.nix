{ inputs

# Dependencies
, main
, mdbook
, stdenv
}:

stdenv.mkDerivation {
  inherit (main) pname version;

  src = inputs.nix-filter {
    root = inputs.self;
    include = [
      "book.toml"
      "conduwuit-example.toml"
      "CONTRIBUTING.md"
      "README.md"
      "debian/README.md"
      "docs"
    ];
  };

  nativeBuildInputs = [
    mdbook
  ];

  buildPhase = ''
    mdbook build
    mv public $out
  '';
}
