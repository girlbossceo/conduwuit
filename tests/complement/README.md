# Complement

## What's that?

Have a look at [its repository](https://github.com/matrix-org/complement).

## How do I use it with conduwuit?

For reproducible results, Complement support in conduwuit uses Nix to run and generate an image.

After installing Nix, you can run either:

- `nix run #.complement-runtime -- ./path/to/logs.jsonl ./path/to/results.jsonl` to build a Complement image, run the tests, and output the logs and results to the specified paths.

- `nix run #.complement-image` to just build a Complement image
