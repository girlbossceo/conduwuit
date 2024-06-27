# Configuration

This chapter describes various ways to configure conduwuit.

## Basics

Conduwuit uses a config file for the majority of the settings. Please refer to the
[example config file](./configuration/examples.md#example-configuration) for all of those settings.
The config file to use can either be specified on the command line when running conduwuit by specifying the
`-c`, `--config` flag. Alternatively, you can use the environment variable `CONDUWUIT_CONFIG` to specify the config
file to used.

## Environment variables

All of the settings that are found in the config file can be specified by using environment variables.
The environment variable names should be all caps and prefixed with `CONDUWUIT_`.
For example, if the setting you are changing is `max_request_size`, then the environment variable to set is
`CONDUWUIT_MAX_REQUEST_SIZE`.
