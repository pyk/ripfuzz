# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a
Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `vm.getEnv` cheatcode to read environment variables as strings:

  ```solidity
  function getEnv(string calldata key) external returns (string memory value);
  function getEnv(string calldata key, string calldata defaultValue)
      external
      returns (string memory value);
  ```

  The single-argument form reverts when the key is missing:

  ```text
  Failed to get environment variable FOO as type string: environment variable not found
  ```

  The two-argument form returns `defaultValue` when the key is missing.

- Automatic `.env` loading from the project directory (defaults to the current
  working directory). Values are available to `vm.getEnv`. Existing process
  environment variables take precedence over `.env`.

- `vm.fork` cheatcode to create or select a remote chain fork:

  ```solidity
  struct ForkConfig {
      uint32 retries;
      uint64 backoffMs;
      uint64 timeoutMs;
      uint64 rateLimit;
  }

  function fork(string calldata url, uint256 blockNumber) external;
  function fork(string calldata url, uint256 blockNumber, ForkConfig config)
      external;
  ```

  Campaigns always start as an empty sandbox. Call `vm.fork` in `setup` or
  action modifiers to opt into remote state. Multiple forks are cached and
  selected by `(url, block)`. Local accounts (harness, deployer, `vm.addr`
  results) persist across switches. Remote state is isolated per fork, so the
  same address on two chains (e.g. a bridge on Ethereum and Polygon) keeps
  independent storage and balances. Coverage is keyed by bytecode hash, not
  address. Single-arg `vm.fork` defaults: retries 3, backoff 100ms, timeout
  30s, no rate limit (same as the former CLI defaults).

### Changed

- `ripfuzz run <HARNESS>` accepts a bare harness name (`Harness`) or a full
  artifact id (`src/Harness.sol:Harness`). When multiple contracts share the
  same name, the command lists the matching full ids to choose from
- Upgraded solc dependency to v0.0.14
- Campaign directory IDs include seconds
  (`.ripfuzz/campaigns/YYYY-MM-DD-HHMMSS-<uuid>/`) so campaigns started in the
  same minute are easier to tell apart
- Fork mode is driven entirely by `vm.fork` in the harness. CLI flags
  `--rpc-url`, `--rpc-block`, `--rpc-retries`, `--rpc-backoff`,
  `--rpc-timeout`, and `--rpc-rate-limit` are removed. Single-arg
  `vm.fork(url, block)` uses built-in defaults (retries 3, backoff 100ms,
  timeout 30s, no rate limit). Override via `vm.fork(url, block, ForkConfig)`

### Fixed

## [0.9.0] - 2026-08-06

Initial public release

### Added

- `ripfuzz run <HARNESS>`: run a coverage-guided, mutational fuzzing campaign
  against a Foundry harness contract using name or `File.sol:Name` artifact ID
  syntax
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
