# Conduit
### A Matrix homeserver written in Rust

[![Liberapay](https://img.shields.io/liberapay/receives/timokoesters?logo=liberapay)](https://liberapay.com/timokoesters)
[![Matrix](https://img.shields.io/matrix/conduit:koesters.xyz?server_fqdn=matrix.koesters.xyz&logo=matrix)](https://matrix.to/#/#conduit:koesters.xyz)

#### Is it fast?

See it in action: <https://www.youtube.com/watch?v=HoBslDtqC1k>

#### What is the goal

A fast Matrix homeserver that's optimized for smaller, personal servers, instead of one server that has high scalability.

#### What is it build on?

- [Ruma](https://www.ruma.io): Useful structures for endpoint requests and responses that can be (de)serialized
- [Sled](https://github.com/spacejam/sled): A simple (key, value) database with good performance
- [Rocket](https://rocket.rs): A flexible web framework

#### What are the next steps?

- [x] Register, login, authentication tokens
- [x] Create room messages
- [x] Sync room messages
- [x] Join rooms, lookup room ids
- [x] Riot web support
- [x] Room discovery
- [x] Read receipts
- [x] Typing indications
- [x] Invites, user search
- [x] Password hashing
- [x] Riot E2EE
- [x] Media
- [x] Permission system
- [ ] Basic federation
- [ ] State resolution
- [ ] Notifications (push rules)
- [ ] Riot presence

#### How can I contribute?

The best way to find something to work on is by joining the #conduit:koesters.xyz Matrix room and asking.

#### Donate

Liberapay: <https://liberapay.com/timokoesters/>
