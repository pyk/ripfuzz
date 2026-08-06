# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a
Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

### Changed

- Upgraded solc dependency to v0.0.14

### Fixed

## [0.9.0] - 2026-08-06

Initial public release

### Added

- `ripfuzz run <target>`: run a coverage-guided, mutational fuzzing campaign
  against a Foundry handler contract using `File.sol:Name` artifact ID syntax
- Parallel fuzzing across configurable worker threads with a shared
  coverage-guided corpus and metrics
- Invariant checking: handler invariant functions are executed after each
  generated call sequence
- Persistent corpus support: `--corpus-dir` loads and replays existing corpus
  items at campaign start and saves newly discovered coverage-increasing
  sequences
- Automatic failure shrinking with configurable `--shrink-runs`,
  `--shrink-timeout`, and `--shrink-threads`
- Fork mode against live networks via `--rpc-url` and `--rpc-block`, with
  retries, exponential backoff, rate limiting, request timeouts, and a local
  fork state cache
- `lcov.info` coverage reports per campaign, including function-level and
  source-map-derived line coverage across all resolved build artifacts
- Execution traces for deployment and setup, written to `trace.log` under
  `.ripfuzz/campaigns/<campaign-id>/`
- Foundry cheatcodes for time and chain context (`warp`, `roll`, `prevrandao`,
  `chain_id`, `coinbase`, `fee`), accounts and balances (`prank`, `deal`,
  `addr`, `nonce`, `label`), storage and bytecode (`store`, `load`, `etch`,
  `get_code`), value encoding/decoding (`parse`, `to_string`, `sign`), and
  opt-in `ffi` via `--ffi`
- `--external-project` to load additional Foundry project artifacts for
  coverage and trace resolution, including fork-mode interactions with
  separately compiled contracts
- Configurable campaign limits: `--max-runs`, `--timeout`, `--max-calls`,
  `--gas-limit`, and `--threads`
- Reproducible campaigns via `--seed`; a random seed is generated and printed
  when none is provided
- `--fail-on-revert` to treat any transaction revert as a failed assertion
- Foundry project integration: automatic builds with storage layout, handler
  deployment with configurable `--deployer` and `--deploy-value`, and library
  linking
- File logging to `fuzz.log` per campaign with configurable `--log-level` and
  `--disable-log`
- Library-first public API with `Fuzzer`, `Shrinker`, `Chain`, `Project`,
  `SharedCorpus`, `CorpusReplayer`, and `CoverageReporter` types for
  programmatic use

[unreleased]: https://github.com/pyk/ripfuzz/compare/v0.9.0...HEAD
[0.9.0]: https://github.com/pyk/ripfuzz/releases/tag/v0.9.0
