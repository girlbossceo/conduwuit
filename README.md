# Conduit
### A Matrix homeserver written in Rust

[![Liberapay](https://img.shields.io/liberapay/receives/timokoesters?logo=liberapay)](https://liberapay.com/timokoesters)
[![Matrix](https://img.shields.io/matrix/conduit:koesters.xyz?server_fqdn=matrix.koesters.xyz&logo=matrix)](https://matrix.to/#/#conduit:koesters.xyz)

#### Goals

A Matrix Homeserver that's faster than others.

#### What is it build on?

- [Ruma](https://www.ruma.io): Useful structures for endpoint requests and responses that can be (de)serialized
- [Sled](https://github.com/spacejam/sled): A simple (key, value) database with good performance
- [Rocket](https://rocket.rs): A flexible web framework

#### Roadmap

- [x] Register, login, authentication tokens
- [x] Create room messages
- [x] Sync room messages
- [x] Join rooms, lookup room ids
- [x] Basic Riot web support
- [x] Riot room discovery
- [x] Riot read receipts
- [x] Typing indications
- [ ] Riot presence
- [ ] Password hashing
- [ ] Proper room creation
- [ ] Riot E2EE
- [ ] Basic federation
- [ ] State resolution

#### Donate

Liberapay: <https://liberapay.com/timokoesters/>
