# conduwuit for NixOS

conduwuit can be acquired by [Lix][lix] from various places:

* The `flake.nix` at the root of the repo
* The `default.nix` at the root of the repo
* From conduwuit's binary cache

A binary cache for conduwuit that the CI/CD publishes to is available at the
following places (both are the same just different names):
```
https://attic.kennel.juneis.dog/conduit
conduit:Isq8FGyEC6FOXH6nD+BOeAA+bKp6X6UIbupSlGEPuOg=

https://attic.kennel.juneis.dog/conduwuit
conduwuit:lYPVh7o1hLu1idH4Xt2QHaRa49WRGSAqzcfFd94aOTw=
```

If specifying a URL in your flake, please use the GitHub remote: `github:girlbossceo/conduwuit`

The `flake.nix` and `default.nix` do not (currently) provide a NixOS module, so
(for now) [`services.matrix-conduit`][module] from Nixpkgs should be used to
configure conduwuit.

If you want to run the latest code, you should get Conduwuit from the `flake.nix`
or `default.nix` and set [`services.matrix-conduit.package`][package]
appropriately.

[lix]: https://lix.systems/
[module]: https://search.nixos.org/options?channel=unstable&query=services.matrix-conduit
[package]: https://search.nixos.org/options?channel=unstable&query=services.matrix-conduit.package
