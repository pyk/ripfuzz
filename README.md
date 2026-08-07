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


## Blog Posts

- [`rvm.fork` instead of `--rpc-url`](https://pyk.sh/blog/2026-08-07-vm-fork-instead-of-cli)
- [Coverage-guided fuzzing with revm](https://pyk.sh/blog/2026-05-28-coverage-guided-fuzzing-with-revm)
- [Replacing my revm `ForkDB` background thread with `SharedBackend`](https://pyk.sh/blog/2026-05-24-forkdb-shared-backend)

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
