# Complement

## What's that?

Have a look at [its repository](https://github.com/matrix-org/complement).

## How do I use it with Conduit?

The script at [`../bin/complement`](../bin/complement) has automation for this.
It takes a few command line arguments:

- Path to Complement's source code
- A `.jsonl` file to write test logs to
- A `.jsonl` file to write test results to

Example: `./bin/complement "../complement" "logs.jsonl" "results.jsonl"`
