# WICRS Server

[![wic.rs](https://img.shields.io/badge/https-wic.rs-green)](https://wic.rs)
[![crates.io](https://img.shields.io/crates/v/wicrs_server.svg)](https://crates.io/crates/wicrs_server)
[![docs.rs](https://docs.rs/wicrs_server/badge.svg)](https://docs.rs/wicrs_server)
[![Discord](https://img.shields.io/discord/822858421064958033?label=discord)](https://discord.gg/dAbjENCdfJ)
[![Matrix](https://img.shields.io/matrix/wicrs:matrix.org?server_fqdn=matrix.org&label=matrix)](https://matrix.to/#/+wicrs:matrix.org)

A server for handling chat rooms and messaging groups written in Rust.

## Build

Install Rust by following [these](https://www.rust-lang.org/tools/install) instructions.
Then clone the git repo, then to build:

```bash
git clone https://github.com/wicrs/server.git wicrs_server
cd wicrs_server
cargo build # to build the release version run cargo build --release
```

## Setup

First you need to create a GitHub OAuth application by following the instructions [here](https://docs.github.com/en/free-pro-team@latest/developers/apps/creating-an-oauth-app), make sure to set the callback URL to `$HOSTNAME:$PORT/api/v2/auth/github`, replace `$PORT` with the port you choose in the config and replace `$HOSTNAME` with the address you will navigate to when accessing the WICRS API.

To run the server you first need to create a config file named `config.json` in the server's working directory, which should be reserved for the server.
Here is an example of what the contents of `config.json` should be:

```json
{
    "key_server": "https://keys.openpgp.org",
    "address": "127.0.0.1:8080",
    "show_version": false,
    "key_id": "WICRS Server <wicrs@example.com>"
}

```

The key server corresponds to the URL of an SKS key server.
`address` should be set to the local address you want the server to listen on, for example you can use `127.0.0.1:8080`. The `show_version` variable determines whether or not the server will tell clients it's version when they go to the HTTP root (`/`). The `key_id` variable optionally pre-configures the ID given to the PGP keys that the server generates (to use a custom PGP key make sure that it is signed and not password protected, then export it as ASCII armour and put it in the file `data/secret_key.asc`).

Note that the server application needs to be able to read `./config.json` and must be able to read and write to `./data` or most if not all requests will fail.

Once this is done run the server by executing `cargo run` or `cargo run --release` if you are in the project git directory. If you are not in the project's git directory you will need to either put the executable in the desired run directory (where you have the `config.json` file) and run `./wicrs_server`. Otherwise you need to have it in your path in which case you just need to run `wicrs_server` in your chosen run directory.

## Developing and Contributing

For information on developing and contributing please read the [contributing guidelines](https://github.com/wicrs/server/blob/master/CONTRIBUTING.md).

## Versioning

This project adheres to [Semantic Versioning](http://semver.org/). However until 1.0.0 comes the following rules apply:

- Any API/ABI breaking changes will result in minor version bumping.
- API extending features results in patch version bumping.
- Non-breaking bug fixes and performance improving results in patch version bumping.
