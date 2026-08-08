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

**Ripfuzz** is a high-throughput, coverage-guided, mutational fuzzer for
Solidity smart contracts. Point it at a harness contract and it generates
stateful call sequences, steers toward new EVM coverage, checks your invariants
after every sequence, and shrinks any `assert` panic it finds into a minimal
reproduction.

## Features

- **Coverage-guided fuzzing**: steer inputs toward unexplored code using
  per-PC edges, call-stack depths, revert paths, and jump destinations.
- **Parallel fuzzing**: one fuzzer per CPU core by default, sharing a
  coverage-guided corpus and metrics across all workers.
- **Lightning fast shrinker**: minimize any failed assertion down to the fewest
  calls that still reproduce it, with shrinking running in parallel across
  multiple workers.
- **Stateful call sequences**: each fuzz input is a sequence of handler calls
  (up to 100 by default), so violations of protocol invariants that only emerge
  through the interaction of multiple calls can be reached instead of just
  single-transaction edge cases.
- **Invariant checking**: run protocol invariants after every call sequence and
  report any violation as a bug.
- **Multi-chain fork mode**: opt into remote chain state with the `rvm.fork`
  cheatcode, with per-fork isolation and harness storage shared across chains
  for cross-chain invariants.
- **Cheatcodes**: manipulate accounts, balances, block context, storage, and
  bytecode from inside the harness, plus environment access and opt-in `ffi`,
  via [ripfuzz-std](https://github.com/pyk/ripfuzz-std).
- **Persistent corpus**: keep interesting sequences between runs, and replay
  them when a new campaign starts.
- **Coverage reports**: per-campaign reports with line and function coverage
  resolved from source maps.
- **Execution traces**: full traces of deployment, setup, and every generated
  call sequence, saved with the campaign.
- **Reproducible runs**: every campaign can be replayed exactly, either from a
  provided seed or from the one printed at start.

## Installation

### From source

```bash
git clone https://github.com/pyk/ripfuzz.git
cd ripfuzz
make bin
```

This runs `cargo install --path . --locked` and installs the `ripfuzz` binary
to your Cargo bin directory.

### Prerequisites

- [Rust](https://rustup.rs/) (edition 2024)
- [Foundry](https://getfoundry.sh/) v1.7.1 or newer, used to compile the
  harness contract

Ripfuzz uses Foundry to compile the harness contract. The project should be set
up with the following in `foundry.toml` so artifacts include the AST and
storage layout:

```toml
[profile.default]
ast = true
extra_output = ["storageLayout"]
```

## Quick start

Write a harness contract with handler functions (any `external`/`public`
function) and invariants (functions prefixed with `invariant_`), then run:

```bash
ripfuzz run src/Harness.sol:Harness
```

For cheatcodes, fork mode, and a full harness reference, see
[`docs/harness-contract.md`](docs/harness-contract.md) and
[`docs/fork-mode.md`](docs/fork-mode.md).

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

## License

MIT
