# Conduit
### A Matrix homeserver written in Rust

[![Liberapay](https://img.shields.io/liberapay/receives/timokoesters?logo=liberapay)](https://liberapay.com/timokoesters)
[![Matrix](https://img.shields.io/matrix/conduit:koesters.xyz?server_fqdn=matrix.koesters.xyz&logo=matrix)](https://matrix.to/#/#conduit:koesters.xyz)

#### What is the goal

A fast Matrix homeserver that's easy to set up and just works. You can install it on a mini-computer like the Raspberry Pi to host Matrix for your family, friends or company.

#### Is it fast?

See it in action: <https://www.youtube.com/watch?v=HoBslDtqC1k>

#### Can I try it out?

Yes! Just open Riot (<https://riot.im/app> or RiotX) and register on the `https://conduit.koesters.xyz:14004` homeserver.

#### How can I deploy my own?

You just have to clone the repo, build it with `cargo build --release` and call the binary (target/release/conduit) from somewhere like a systemd script.
It's explained in more detail [here](https://git.koesters.xyz/timo/conduit/wiki/Deploy).

#### What is it build on?

- [Ruma](https://www.ruma.io): Useful structures for endpoint requests and responses that can be (de)serialized
- [Sled](https://github.com/spacejam/sled): A simple (key, value) database with good performance
- [Rocket](https://rocket.rs): A flexible web framework

#### What are the biggest things still missing?

- Federation (Make Conduit talk to other Matrix servers)
- Notifications (Make Matrix client notify the user when new messages or pings arrive)
- Lots of testing

Also check out the [milestones](https://git.koesters.xyz/timo/conduit/milestones).

#### How can I contribute?

If you want to help, you may be able to find something in the issue tracker. If you do, comment on the issue, so others know. You can also join #conduit:matrix.org and ask there.

#### Donate

Liberapay: <https://liberapay.com/timokoesters/>
Bitcoin: `bc1qnnykf986tw49ur7wx9rpw2tevpsztvar5x8w4n`
