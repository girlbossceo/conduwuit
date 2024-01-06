# conduwuit
### a well maintained fork of [Conduit](https://conduit.rs/)

#### What is Matrix?
[Matrix](https://matrix.org) is an open network for secure and decentralized
communication. Users from every Matrix homeserver can chat with users from all
other Matrix servers. You can even use bridges (also called Matrix appservices)
to communicate with users outside of Matrix, like a community on Discord.

#### What is the goal?

An efficient Matrix homeserver that's easy to set up and just works. You can install
it on a mini-computer like the Raspberry Pi to host Matrix for your family,
friends or company.

#### Can I try it out?

There are no public conduwuit homeservers available, however conduwuit is incredibly simple to install. It's just a binary, a config file, and a database path.

#### What is the current status?

conduwuit is a fork of Conduit which is in beta, meaning you can join and participate in most
Matrix rooms, but not all features are supported and you might run into bugs
from time to time. conduwuit attempts to fix and improve the majority of upstream Conduit bugs
or UX issues that are taking too long to be resolved, or unnecessary Matrix or developer
politics halting simple things from being merged or fixed, and general inactivity.

There are still a few nice to have features missing that some users may notice:

- Outgoing read receipts and typing indicators (receiving works)

#### What's different about your fork than upstream Conduit?

See [DIFFERENCES.md](DIFFERENCES.md)

#### Why does this fork exist? Why don't you contribute back upstream?

I have tried, but:
- unnecessary Matrix / developer politics
- bikeshedding unnecessary or irrelevant things in MRs
- disagreement with how the upstream project is maintained including the codebase
- infinitely broken CI/CD and no interest in fixing it or improving it
- upstream maintainer inactivity
- questionable community members
- lack of MR reviews or issue triaging and no upstream maintainer interest in receiving help
- severe bugs, including denial of service and other likely vulnerabilities, not being merged due to things mentioned above
- no interest in adding co-maintainers to help out

are what are keeping me from contributing. If the state of the upstream project improves, I'm
willing to start contributing again. As is, I think if folks want a more polished and well-kept version of Conduit, conduwuit exists for that.

#### How can I deploy my own?

- Simple install (this was tested the most): [DEPLOY.md](DEPLOY.md)
- Nix/NixOS: [nix/README.md](nix/README.md)

If you want to connect an Appservice to Conduit, take a look at [APPSERVICES.md](APPSERVICES.md).

#### How can I contribute?

1. Look for an issue you would like to work on and make sure it's not assigned
   to other users
2. Ask someone to assign the issue to you (comment on the issue or chat in
   [#conduwuit:puppygock.gay](https://matrix.to/#/#conduwuit:puppygock.gay))
3. Fork the repo and work on the issue.
4. Submit a PR (please keep contributions to the GitHub repo, main development is done here,
not the GitLab repo which exists just as a mirror.)

#### Contact

If you run into any question, feel free to
- Ask us in `#conduwuit:puppygock.gay` on Matrix
- [Open an issue on GitHub](https://github.com/girlbossceo/conduwuit/issues/new)

#### Donate

Liberapay: <https://liberapay.com/girlbossceo>\
Ko-fi: <https://ko-fi.com/puppygock>\
GitHub Sponsors: <https://github.com/sponsors/girlbossceo>


#### Logo

No official conduwuit logo exists. Repo and Matrix room picture is from bran (<3).
