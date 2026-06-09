# raptor

Parallelized, coverage-guided, mutational Solidity smart contract fuzzing,
powered by revm.

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

## Known issues

### Fuzzer may hang on out-of-gas revert loops

By default raptor sets the block gas limit, transaction gas limit, and
call/deploy gas limit to `u64::MAX` (effectively unlimited). If the target
contract logic depends on gas metering (e.g., loops or recursion guarded by a
`gasleft()` check), the fuzzer can enter an infinite execution where the
contract never reverts with `OutOfGas` and instead runs forever.

**Workaround:** pass an explicit gas limit when constructing deploy or call
inputs.
