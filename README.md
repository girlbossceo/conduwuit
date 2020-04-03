# Matrix Homeserver in Rust

#### Goals

A Matrix Homeserver that's faster than others.

#### What is it build on?

- [Ruma](https://www.ruma.io): Useful structures for endpoint requests and responses that can be (de)serialized
- [Sled](https://github.com/spacejam/sled): A simple (key, value) database with good performance
- [Rocket](https://rocket.rs): A flexible web framework

#### Roadmap

- [x] Register, login, authentication tokens
- [ ] Create room messages
- [ ] Sync room messages
- [ ] Join rooms, lookup room ids
