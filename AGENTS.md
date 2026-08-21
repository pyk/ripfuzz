# Ripfuzz

Ripfuzz is a high-throughput, coverage-guided, mutational fuzzer for Solidity
smart contracts.

## Non-negotiable rules

List of CRITICAL rules that you must follow every time. Failing to do so will
have a severe negative impact on the project and the user.

### General Rules

| ID     | Rule                                                         |
| :----- | :----------------------------------------------------------- |
| GEN-01 | MUST run `make lint` and `make test` before finishing a task |
| GEN-02 | MUST use `cargo txt` to view crate documentation             |

### Code Design Rules

| ID      | Rule                                                                                                                                                     |
| :------ | :------------------------------------------------------------------------------------------------------------------------------------------------------- |
| CODE-01 | MUST separate I/O from logic                                                                                                                             |
| CODE-02 | MUST NOT add comment block headers                                                                                                                       |
| CODE-03 | MUST design the public API around types, not functions                                                                                                   |
| CODE-04 | MUST organize modules around domain concepts                                                                                                             |
| CODE-05 | MUST keep one primary type per module (`fuzzer.rs` -> Fuzzer). MUST NOT use `mod.rs`; a module with children lives at `foo.rs` with submodules in `foo/` |
| CODE-06 | MUST re-export public types at the module level                                                                                                          |
| CODE-07 | MUST keep implementation details private                                                                                                                 |
| CODE-08 | MUST NOT create `utils.rs`, `helpers.rs`, or `common.rs`                                                                                                 |
| CODE-09 | MUST put behavior on the type that owns the state                                                                                                        |
| CODE-10 | MUST use constructors as entry points (e.g. `Project::open(path)`)                                                                                       |
| CODE-11 | MUST NOT prefix function names with the type name (bad: `build_project`)                                                                                 |
| CODE-12 | MUST use free functions only when there is no natural owner                                                                                              |
| CODE-13 | MUST use option structs for methods with many parameters                                                                                                 |
| CODE-14 | MUST use operation types (Analyzer, Linker, Builder) for complex workflows                                                                               |
| CODE-15 | MUST use context objects for internal workflows to prevent parameter explosion                                                                           |
| CODE-16 | MUST avoid deep module hierarchies                                                                                                                       |
| CODE-17 | MUST put code snippets in module docs inside fenced code blocks, not inline backticks                                                                    |

### Testing Rules

| ID      | Rule                                                                                                      |
| :------ | :-------------------------------------------------------------------------------------------------------- |
| TEST-01 | MUST assert exact error messages in tests with `assert_eq!`, never `.contains()`                          |
| TEST-02 | MUST NOT add trivial tests. Tests must assert real behavior, not pure formatting/parsing/encoding helpers |

### Changelog Rules

| ID        | Rule                                                                                                                             |
| :-------- | :------------------------------------------------------------------------------------------------------------------------------- |
| CHANGE-01 | MUST follow Keep a Changelog and Semantic Versioning                                                                             |
| CHANGE-02 | MUST record user-visible changes under `## [Unreleased]`                                                                         |
| CHANGE-03 | MUST use the subsections `### Added`, `### Changed`, and `### Fixed` under `[Unreleased]` (leave a subsection empty when unused) |
| CHANGE-04 | MUST move `[Unreleased]` entries into a new versioned section `## [X.Y.Z] - YYYY-MM-DD` when preparing a release                 |
| CHANGE-05 | MUST keep an empty `[Unreleased]` section at the top with `### Added`, `### Changed`, and `### Fixed` after cutting a release    |
| CHANGE-06 | MUST update the comparison links at the bottom of `CHANGELOG.md` when adding a new version section                               |

After cutting a release section, the empty `[Unreleased]` block MUST look like:

```md
## [Unreleased]

### Added

### Changed

### Fixed

## [X.Y.Z] - YYYY-MM-DD
```

### Commit Rules

| ID        | Rule                                                                                                                                   |
| :-------- | :------------------------------------------------------------------------------------------------------------------------------------- |
| COMMIT-01 | MUST write commit messages in Conventional Commits format: `<type>(<scope>): <subject>` (e.g. `feat(dataset): ...`, `chore(git): ...`) |
| COMMIT-02 | MUST keep the subject compact: lower-case after the colon, no trailing period, and wrapped to about 72 columns                         |
| COMMIT-03 | MUST add a wrapped body paragraph (about 72 columns) explaining the why for non-trivial changes; skip the body for trivial changes     |
| COMMIT-04 | MUST NOT create a commit unless the user explicitly asks for one                                                                       |
| COMMIT-05 | MUST stop and ask the user when a commit surfaces an interactive prompt (e.g. a GPG passphrase)                                        |
| COMMIT-06 | MUST NOT type a passphrase or other secret into an interactive commit prompt                                                           |
| COMMIT-07 | MUST NOT bypass signing to force the commit through with `-c commit.gpgsign=false`                                                     |

## Tool References

### cargo txt

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

### checkrs

To suppress the lint use the `// checkrs: allow(<name>)` for example:

```rust
// checkrs: allow(clone_in_loops)
let mut fresh_chain = self.chain.clone();
```

or

```rust
let mut fresh_chain = self.chain.clone(); // checkrs: allow(clone_in_loops)
```
