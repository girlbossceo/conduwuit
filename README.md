# Conduit
### A Matrix homeserver written in Rust

[![Liberapay](https://img.shields.io/liberapay/receives/timokoesters?logo=liberapay)](https://liberapay.com/timokoesters)
[![Matrix](https://img.shields.io/matrix/conduit:koesters.xyz?server_fqdn=matrix.koesters.xyz&logo=matrix)](https://matrix.to/#/#conduit:koesters.xyz)

#### What is the goal?

A fast Matrix homeserver that's easy to set up and just works. You can install
it on a mini-computer like the Raspberry Pi to host Matrix for your family,
friends or company.

#### Can I try it out?

Yes! Just open a Matrix client (<https://app.element.io> or Element Android for
example) and register on the `https://conduit.koesters.xyz` homeserver.

#### How can I deploy my own?

##### From source

Clone the repo, build it with `cargo build --release` and call the binary
(target/release/conduit) from somewhere like a systemd script. [Read
more](DEPLOY_FROM_SOURCE.md)

##### Using Docker

Pull and run the docker image with

``` bash
docker pull matrixconduit/matrix-conduit:latest
docker run -d matrixconduit/matrix-conduit:latest -p 8448:8000 -v db:/srv/conduit/.local/share/conduit
```

Or build and run it with docker or docker-compose. [Read more](docker/README.md)

#### What is it build on?

- [Ruma](https://www.ruma.io): Useful structures for endpoint requests and
  responses that can be (de)serialized
- [Sled](https://github.com/spacejam/sled): A simple (key, value) database with
  good performance
- [Rocket](https://rocket.rs): A flexible web framework

#### What are the biggest things still missing?

- Appservices (Bridges and Bots)
- Most federation features (invites, e2ee)
- Push notifications on mobile
- Notification settings
- Lots of testing

Also check out the [milestones](https://git.koesters.xyz/timo/conduit/milestones).

#### How can I contribute?

1. Look for an issue you would like to work on and make sure it's not assigned
   to other users
2. Ask someone to assign the issue to you (comment on the issue or chat in
   #conduit:matrix.org)
3. Fork the repo and work on the issue. #conduit:matrix.org is happy to help :)
4. Submit a PR

#### Donate

Liberapay: <https://liberapay.com/timokoesters/>\
Bitcoin: `bc1qnnykf986tw49ur7wx9rpw2tevpsztvar5x8w4n`
