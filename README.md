<h3 align="center">
    ripfuzz
</h3>

<p align="center">
    High-throughput, coverage-guided, mutational fuzzer for Solidity smart contracts.
<p>

<p align="center">
  <img src="https://img.shields.io/crates/v/ripfuzz.svg?colorA=00f&colorB=fff&style=flat&logo=rust" alt="Crates.io">
  <img src="https://img.shields.io/crates/d/ripfuzz?colorA=00f&colorB=fff&style=flat&logo=rust" alt="Downloads">
  <img src="https://img.shields.io/github/license/pyk/ripfuzz?colorA=00f&colorB=fff&style=flat" alt="MIT License">
</p>

> [!IMPORTANT]
>
> `ripfuzz` is in early active development.


## Development

Install the binary locally from source:

```sh
cargo install --path .
```

Build and run without installing:

```sh
cargo run -- --help
```

Run the test suite:

```sh
cargo test

# Run integration tests for network forking
RIPFUZZ_FORK_RPC_URL=<url> cargo test -- --ignored
```

Check code with Clippy:

```sh
cargo clippy
```
