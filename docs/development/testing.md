# Testing

## Complement

Have a look at [Complement's repository][complement] for an explanation of what
it is.

To test against Complement, with Nix and direnv installed and set up, you can
either:

* Run `complement "$COMPLEMENT_SRC" ./path/to/logs.jsonl ./path/to/results.jsonl`
  to build a Complement image, run the tests, and output the logs and results
  to the specified paths
* Run `nix build .#complement` from the root of the repository to just build a
  Complement image

[complement]: https://github.com/matrix-org/complement
