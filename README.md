# conduwuit

[![conduwuit main room](https://img.shields.io/matrix/conduwuit%3Apuppygock.gay?server_fqdn=matrix.transfem.dev&style=flat&logo=matrix&logoColor=%23f5b3ff&label=%23conduwuit%3Apuppygock.gay&color=%23f652ff)](https://matrix.to/#/#conduwuit:puppygock.gay) [![conduwuit space](https://img.shields.io/matrix/conduwuit-space%3Apuppygock.gay?server_fqdn=matrix.transfem.dev&style=flat&logo=matrix&logoColor=%23f5b3ff&label=%23conduwuit-space%3Apuppygock.gay&color=%23f652ff)](https://matrix.to/#/#conduwuit-space:puppygock.gay) [![CI and Artifacts](https://github.com/girlbossceo/conduwuit/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/girlbossceo/conduwuit/actions/workflows/ci.yml)

<!-- ANCHOR: catchphrase -->

### a very cool [Matrix](https://matrix.org/) chat homeserver written in Rust

<!-- ANCHOR_END: catchphrase -->

Visit the [conduwuit documentation](https://conduwuit.puppyirl.gay/) for more
information and how to deploy/setup conduwuit.

<!-- ANCHOR: body -->

#### What is Matrix?

[Matrix](https://matrix.org) is an open, federated, and extensible network for
decentralised communication. Users from any Matrix homeserver can chat with users from all
other homeservers over federation. Matrix is designed to be extensible and built on top of.
You can even use bridges such as Matrix Appservices to communicate with users outside of Matrix, like a community on Discord.

#### What is the goal?

A high-performance, efficient, low-cost, and featureful Matrix homeserver that's
easy to set up and just works with minimal configuration needed.

#### Can I try it out?

An official conduwuit server ran by me is available at transfem.dev
([element.transfem.dev](https://element.transfem.dev) /
[cinny.transfem.dev](https://cinny.transfem.dev))

transfem.dev is a public homeserver that can be used, it is not a "test only
homeserver". This means there are rules, so please read the rules:
[https://transfem.dev/homeserver_rules.txt](https://transfem.dev/homeserver_rules.txt)

transfem.dev is also listed at
[servers.joinmatrix.org](https://servers.joinmatrix.org/), which is a list of
popular public Matrix homeservers, including some others that run conduwuit.

#### What is the current status?

conduwuit is technically a hard fork of [Conduit](https://conduit.rs/), which is in beta.
The beta status initially was inherited from Conduit, however the huge amount of
codebase divergance, changes, fixes, and improvements have effectively made this
beta status not entirely applicable to us anymore.

conduwuit is very stable based on our rapidly growing userbase, has lots of features that users
expect, and very usable as a daily driver for small, medium, and upper-end medium sized homeservers.

A lot of critical stability and performance issues have been fixed, and a lot of
necessary groundwork has finished; making this project way better than it was
back in the start at ~early 2024.

#### How is conduwuit funded? Is conduwuit sustainable?

conduwuit has no external funding. This is made possible purely in my freetime with
contributors, also in their free time, and only by user-curated donations.

conduwuit has existed since around November 2023, but [only became more publicly known
in March/April 2024](https://matrix.org/blog/2024/04/26/this-week-in-matrix-2024-04-26/#conduwuit-website)
and we have no plans in stopping or slowing down any time soon!

#### Can I migrate or switch from Conduit?

conduwuit is a complete drop-in replacement for Conduit. As long as you are using RocksDB,
the only "migration" you need to do is replace the binary or container image. There
is no harm or additional steps required for using conduwuit. See the
[Migrating from Conduit](https://conduwuit.puppyirl.gay/deploying/generic.html#migrating-from-conduit) section
on the generic deploying guide.

Note that as of conduwuit version 0.5.0, backwards compatibility with Conduit is
no longer supported. We only support migrating *from* Conduit, not back to
Conduit like before. If you are truly finding yourself wanting to migrate back
to Conduit, we would appreciate all your feedback and if we can assist with
any issues or concerns.

#### Can I migrate from Synapse or Dendrite?

Currently there is no known way to seamlessly migrate all user data from the old
homeserver to conduwuit. However it is perfectly acceptable to replace the old
homeserver software with conduwuit using the same server name and there will not
be any issues with federation.

There is an interest in developing a built-in seamless user data migration
method into conduwuit, however there is no concrete ETA or timeline for this.


<!-- ANCHOR_END: body -->

<!-- ANCHOR: footer -->

#### Contact

[`#conduwuit:puppygock.gay`](https://matrix.to/#/#conduwuit:puppygock.gay)
is the official project Matrix room. You can get support here, ask questions or
concerns, get assistance setting up conduwuit, etc.

This room should stay relevant and focused on conduwuit. An offtopic general
chatter room can be found there as well.

Please keep the issue trackers focused on bug reports and enhancement requests.
General support is extremely difficult to be offered over an issue tracker, and
simple questions should be asked directly in an interactive platform like our
Matrix room above as they can turn into a relevant discussion and/or may not be
simple to answer. If you're not sure, just ask in the Matrix room.

If you have a bug or feature to request: [Open an issue on GitHub](https://github.com/girlbossceo/conduwuit/issues/new)

#### Donate

conduwuit development is purely made possible by myself and contributors. I do
not get paid to work on this, and I work on it in my free time. Donations are
heavily appreciated! 💜🥺

- Liberapay (preferred): <https://liberapay.com/girlbossceo>
- GitHub Sponsors (preferred): <https://github.com/sponsors/girlbossceo>
- Ko-fi: <https://ko-fi.com/puppygock>

I do not and will not accept cryptocurrency donations, including things related.

#### Logo

Original repo and Matrix room picture was from bran (<3). Current banner image
and logo is directly from [this cohost
post](https://web.archive.org/web/20241126004041/https://cohost.org/RatBaby/post/1028290-finally-a-flag-for).

#### Is it conduwuit or Conduwuit?

Both, but I prefer conduwuit.

#### Mirrors of conduwuit

If GitHub is unavailable in your country, or has poor connectivity, conduwuit's
source code is mirrored onto the following additional platforms I maintain:

- GitHub: <https://github.com/girlbossceo/conduwuit>
- GitLab: <https://gitlab.com/conduwuit/conduwuit>
- git.girlcock.ceo: <https://git.girlcock.ceo/strawberry/conduwuit>
- git.gay: <https://git.gay/june/conduwuit>
- mau.dev: <https://mau.dev/june/conduwuit>
- Codeberg: <https://codeberg.org/arf/conduwuit>
- sourcehut: <https://git.sr.ht/~girlbossceo/conduwuit>

<!-- ANCHOR_END: footer -->
