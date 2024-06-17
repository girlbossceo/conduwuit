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
      "debian/conduwuit.service"
      "debian/README.md"
      "arch/conduwuit.service"
      "docs"
      "theme"
    ];
  };

  nativeBuildInputs = [
    mdbook
  ];

  buildPhase = ''
    mdbook build -d $out
  '';
}
