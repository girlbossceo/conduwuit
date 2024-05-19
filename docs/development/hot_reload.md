## Hot Reloading ("Live" Development)

### Summary

When doing development/debug builds with the nightly toolchain, conduwuit is modular using dynamic libraries and various parts of the application are hot reloadable such as the APIs, database, service, admin room, RocksDB, etc. These are all split up into individual workspace crates. This design also only reloads the libraries that actually changed. [hot_lib_reload][6] was considered during early research of this, however its too limited in its functionality for conduwuit's use-case where we have *many* things going on (e.g. async code, tower/tower-http, axum, hyper, hickory_resolver, ruma, RocksDB, admin room, etc etc etc), so a custom solution using `libloading` was required.

Release builds are unaffected and still function as fully normal static binaries. Some bugs such as shutdown issues have been fixed as apart of this implementation, but there is no functional difference with release builds as apart of this. You cannot hot reload release binaries.

Currently, this development setup only works on x86_64 and aarch64 Linux glibc. [musl explicitly does not support hot reloadable libraries, and does not implement `dlclose`][2]. macOS does not fully support our usage of `RTLD_GLOBAL` possibly due to some thread-local issues. [This Rust issue][3] may be of relevance, specifically [this comment][4]. It may be possible to get it working on only very modern macOS versions such as at least Sonoma, as currently loading dylibs is supported, but not unloading them in our setup, and the cited comment mentions an Apple WWDC confirming there have been TLS changes to somewhat make this possible.

Additionally, at the time of writing `io_uring` and `jemalloc` are unstable at runtime when using hot reloading. Release builds are not affected.

### Requirements

As mentioned above this requires the nightly toolchain. This is due to reliance on various Cargo.toml features that are only available on nightly, most specifically `RUSTFLAGS` in Cargo.toml. Some of the implementation could also be simpler based on other various nightly features. We hope lots of nightly features start making it out of nightly sooner as there have been dozens of very helpful features that have been stuck in nightly ("unstable") for at least 5+ years that would make this a lot more cleaner, simpler, etc and have worked perfectly fine in testing.

This currently only works on x86_64/aarch64 Linux with a glibc C library. musl C library, macOS, and likely other host architectures are not supported (if other architectures work, feel free to let us know and/or make a PR updating this).

This should work on GNU ld and lld (rust-lld) and gcc/clang, however if you happen to have linker issues it's recommended to try using `mold` or `gold` linkers, and please let us know in the [conduwuit Matrix room][7] the linker error and what linker solved this issue so we can figure out a solution. Ideally there should be minimal friction to using this, and in the future a build script (`build.rs`) may be suitable to making this easier to use if the capabilities allow us.

### Usage

As of 19 May 2024, the instructions for using this are:

0. Have patience. Don't feel hesitated to join the [conduwuit Matrix room][7] to receive help in using this. As indicated by the various rustflags used and some of the interesting issues linked at the bottom, this is definitely not something the Rust ecosystem or toolchain is used to doing.

1. Install the nightly toolchain using rustup. You may need to use `rustup override set nightly` in your local conduwuit directory, or use `cargo +nightly` for all actions.

2. Uncomment `cargo-features` at the top level / root Cargo.toml

3. Scroll down to the `# Developer profile` section and uncomment ALL the rustflags for each dev profile and their respective packages.

4. In each workspace crate's Cargo.toml (everything under `src/*` AND `deps/rust-rocksdb/Cargo.toml`), uncomment the `dylib` crate type under `[lib]`.

5. Due to [this rpath issue][5], you must export the `LD_LIBRARY_PATH` environment variable to your nightly Rust toolchain library directory. If using rustup (hopefully), use this: `export LD_LIBRARY_PATH=$LD_LIBRARY_PATH:$HOME/.rustup/toolchains/nightly-x86_64-unknown-linux-gnu/lib/`

6. Start the server. You can use `cargo +nightly run` for this along with the standard.

7. Make some changes where you need to.

8. In a separate terminal window in the same directory (or using a terminal multiplexer like tmux), run the *build* Cargo command `cargo +nightly build`. Cargo should only rebuild what was changed / what's necessary, so it should not be rebuilding all the crates.

9. In your conduwuit server terminal, hit/send `CTRL+C` signal. This will tell conduwuit to find which libraries need to be reloaded, and reloads them as necessary.

10. If there were no errors, it will tell you it successfully reloaded `#` modules, and your changes should now be visible. Repeat 7 - 9 as needed.

To shutdown conduwuit in this setup, hit/send `CTRL+\`. Normal builds still shutdown with `CTRL+C` as usual.

Steps 1 - 5 are the initial first-time steps for using this. To remove the hot reload setup, revert/comment all the Cargo.toml changes.

As mentioned in the requirements section, if you happen to have some linker issues, try using the `-fuse-ld=` rustflag and specify mold or gold in all the `rustflags` definitions in the top level Cargo.toml, and please let us know in the [conduwuit Matrix room][7] the problem. mold can be installed typically through your distro, and gold is provided by the binutils package.

It's possible a helper script can be made to do all of this, or most preferably a specially made build script (build.rs). `cargo watch` support will be implemented soon which will eliminate the need to manually run `cargo build` all together.

### Design

The initial implementation PR is available [here][1].

TODO: document a lot more stuff about all the design here

![conduwuit's dynamic library setup diagram - created by Jason Volk](assets/libraries.png)

![conduwuit's reload and load order diagram - created by Jason Volk](assets/reload_order.png)

### Interesting related issues/bugs

- [DT_RUNPATH produced in binary with rpath = true is wrong (cargo)][5]
- [Disabling MIR Optimization in Rust Compilation (cargo)](https://internals.rust-lang.org/t/disabling-mir-optimization-in-rust-compilation/19066/5)
- [Workspace-level metadata (cargo-deb)](https://github.com/kornelski/cargo-deb/issues/68)

[1]: https://github.com/girlbossceo/conduwuit/pull/387
[2]: https://wiki.musl-libc.org/functional-differences-from-glibc.html#Unloading-libraries
[3]: https://github.com/rust-lang/rust/issues/28794
[4]: https://github.com/rust-lang/rust/issues/28794#issuecomment-368693049
[5]: https://github.com/rust-lang/cargo/issues/12746
[6]: https://docs.rs/hot-lib-reloader/latest/hot_lib_reloader/
[7]: https://matrix.to/#/#conduwuit:puppygock.gay
