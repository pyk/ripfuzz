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
```

Check code with Clippy:

```sh
cargo clippy
```
