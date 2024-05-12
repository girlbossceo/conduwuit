# Testing

## Complement

Have a look at [Complement's repository][complement] for an explanation of what
it is.

To test against Complement, with Nix and direnv installed and set up, you can:

* Run `./bin/complement "$COMPLEMENT_SRC" ./path/to/logs.jsonl ./path/to/results.jsonl`
  to build a Complement image, run the tests, and output the logs and results
  to the specified paths. This will also output the OCI image at `result`
* Run `nix build .#complement` from the root of the repository to just build a
  Complement OCI image outputted to `result` (it's a `.tar.gz` file)
* Or download the latest Complement OCI image from the CI workflow artifacts output
  from the commit/revision you want to test (e.g. from main) [here][ci-workflows]

[ci-workflows]: https://github.com/girlbossceo/conduwuit/actions/workflows/ci.yml?query=event%3Apush+is%3Asuccess+actor%3Agirlbossceo
[complement]: https://github.com/matrix-org/complement
