# Running Conduit on Complement

This assumes that you're familiar with complement, if not, please readme
[their readme](https://github.com/matrix-org/complement#running).

Complement works with "base images", this directory (and Dockerfile) helps build the conduit complement-ready docker
image.

To build, `cd` to the base directory of the workspace, and run this:

`docker build -t complement-conduit:dev -f complement/Dockerfile .`

Then use `complement-conduit:dev` as a base image for running complement tests.
