# raptor

Parallelized, coverage-guided, mutational Solidity smart contract fuzzing,
powered by LibAFL and revm.

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
RAPTOR_FORK_RPC_URL=<url> cargo test -- --ignored
```

Check code with Clippy:

```sh
cargo clippy
```
