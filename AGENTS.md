# Raptor

Raptor is a parallelized, coverage-guided, mutational Solidity smart contract fuzzer
built on top of revm.

# Architecture & Conventions

You must treat the following documentation as authoritative when modifying raptor:

- [Handler Contract](docs/handler-contract.md) — You must understand these conventions
  because raptor's contract parser, ABI classifier, and fuzzer core are built around
  them.
- [Glossary](docs/glossary.md) — You must use the canonical terms defined here
  (campaign, handler contract, property, action, setup) when writing code or
  documentation.

# Cargo Docs

You must use `cargo txt` to access crate documentation locally.

You must follow this workflow:

1. Build documentation: `cargo txt build <crate>`
2. List all items: `cargo txt list <lib_name>`
3. View a specific item: `cargo txt show <lib_name>::<item>`

For example:

```sh
# Build the serde crate documentation
cargo txt build serde

# List all items in serde
cargo txt list serde

# View serde crate overview
cargo txt show serde

# View serde::Deserialize trait documentation
cargo txt show serde::Deserialize
```
