# conduwuit for NixOS

conduwuit can be acquired by Nix (or [Lix][lix]) from various places:

* The `flake.nix` at the root of the repo
* The `default.nix` at the root of the repo
* From conduwuit's binary cache

A community maintained NixOS package is available at [`conduwuit`](https://search.nixos.org/packages?channel=unstable&show=conduwuit&from=0&size=50&sort=relevance&type=packages&query=conduwuit)

A binary cache for conduwuit that the CI/CD publishes to is available at the
following places (both are the same just different names):

```
https://attic.kennel.juneis.dog/conduit
conduit:eEKoUwlQGDdYmAI/Q/0slVlegqh/QmAvQd7HBSm21Wk=

https://attic.kennel.juneis.dog/conduwuit
conduwuit:BbycGUgTISsltcmH0qNjFR9dbrQNYgdIAcmViSGoVTE=
```

The binary caches were recreated some months ago due to attic issues. The old public
keys were:

```
conduit:Isq8FGyEC6FOXH6nD+BOeAA+bKp6X6UIbupSlGEPuOg=
conduwuit:lYPVh7o1hLu1idH4Xt2QHaRa49WRGSAqzcfFd94aOTw=
```


If specifying a Git remote URL in your flake, you can use any remotes that
are specified on the README (the mirrors), such as the GitHub: `github:girlbossceo/conduwuit`

The `flake.nix` and `default.nix` do not currently provide a NixOS module (contributions
welcome!), so [`services.matrix-conduit`][module] from Nixpkgs can be used to configure
conduwuit.

If you want to run the latest code, you should get conduwuit from the `flake.nix`
or `default.nix` and set [`services.matrix-conduit.package`][package]
appropriately to use conduwuit instead of Conduit.


[lix]: https://lix.systems/
[module]: https://search.nixos.org/options?channel=unstable&query=services.matrix-conduit
[package]: https://search.nixos.org/options?channel=unstable&query=services.matrix-conduit.package
