# Ripfuzz

Ripfuzz is a high-throughput, coverage-guided, mutational fuzzer for Solidity
smart contracts.

## Non-negotiable rules

List of CRITICAL rules that you must follow every time. Failing to do so will
have a severe negative impact on the project and the user.

### General Rules

- You must run `make lint` and `make test` before finishing a task.
- You must not use `cargo doc` to get crate documentation, use `cargo txt` to
  view crate documentation.
- You must not create fixture artifacts manually. Run `forge build` to generate
  them.

### Code Design Rules

- You must separate I/O from logic.
- You must not add comment block header.
- You must design the public API around types, not functions.
- You must organize modules around domain concepts.
- You must keep one primary type per module (`project.rs` -> Project).
- You must re-export public types at the module level.
- You must keep implementation details private.
- You must not create `utils.rs`, `helpers.rs`, or `common.rs`.
- You must put behavior on the type that owns the state.
- You must use constructors as entry points (e.g. `Project::open(path)`).
- You must not prefix function names with the type name (bad: `build_project`).
- You must use free functions only when there is no natural owner.
- You must use option structs for methods with many parameters.
- You must use operation types (Analyzer, Linker, Builder) for complex
  workflows.
- You must use context objects for internal workflows to prevent parameter
  explosion.
- You must avoid deep module hierarchies.

## Cargo Docs

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
