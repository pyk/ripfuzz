# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a
Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

### Changed

- Fork-mode campaigns now throttle RPC batches to a conservative default of 10
  batches per second so default runs stay under public-provider rate quotas;
  override per fork with `vm.fork(..., ForkConfig{rateLimit: N})` or disable
  with `rateLimit: 0`
- The RPC batch retry loop now logs a warning with the retry number, total
  retries, backoff duration, endpoint, request payload, and error as structured
  fields
- Per-thread fuzzer `run` spans now include `fuzzer_id`, so nested logs such as
  RPC retry warnings identify which fuzzer emitted them

### Fixed

- Provider rate-limit (429) and 5xx JSON-RPC error objects inside batch
  responses are now retried with capped exponential backoff instead of killing
  the fuzzer thread immediately
- Maxxing progress and finished logs now always include `value`, showing `0`
  until a non-zero best is found

## [0.9.4] - 2026-08-23

### Added

- Trace decoding extracts function argument types from runtime bytecode with
  evmole when no project ABI matches the selector, so unverified and forked
  calls render decoded arguments instead of `0xselector(...)`

### Changed

- Max-mode campaigns now log under a `maxxing{threads=N}` span instead of
  `fuzz`, and their progress/finished summaries include the current best max
  `value`
- The shrink progress line now logs structured fields (`runs`, `calls`,
  `elapsed`, `call_rate`, `gas_rate`, `initial_calls`, `current_calls`),
  matching the fuzz progress summary
- Max-mode campaigns no longer print the full call sequence in the log; it
  stays available in the trace file
- Harnesses may declare an optional `summary()` function (no arguments, not
  view/pure) that ripfuzz calls once after shrinking in the traced re-run, so
  it can log a final summary that shows up at the end of the trace
- The `Found N distinct failed assertion(s)` finding now logs at `info` instead
  of `error`, since the campaign still completes successfully
- Shrunk invariant failures are now persisted to the corpus, so the next
  campaign discovers the shortest failing sequence during replay instead of
  re-fuzzing it
- Campaign logs now use `shrink{threads=N}` and `trace` spans after fuzzing, so
  the whole lifecycle reads `build` → `deploy` → `replay` → `fuzz` → `shrink` →
  `trace`; shrink progress messages also log `assertion`, `initial_calls`, and
  `final_calls` as structured fields instead of concatenated text
- The `trace` span now prints only the decoded log entries (when present) below
  the `trace:` line and the trace file path, instead of the compact trace and a
  `fulltrace:` line; the campaign log path moves to its own `log` span
- Coverage report generation now logs under a `report` span, the percentage
  line is no longer indented, and the lcov path is full like the trace and log
  paths
- Session setup logs are now grouped under `build`, `deploy{contract=...}`, and
  `replay{items=N}` spans, matching the `fuzz{threads=N}` span
- The `Loaded harness contract` and `Deployed` messages no longer repeat the
  contract name, since the `deploy` span already carries it
- The corpus replay and fuzz progress/finished summaries now collapse the
  edge/depth/revert/jump counters into a single compact `coverage` field (e.g.
  `8,407e 1,409d 17r 782j`) alongside the contract count
- Artifact parse warnings now render as `failed to parse artifact <path>` under
  the `build:load_artifacts` span, without the repeated project path

### Fixed

- Remove the redundant `contract` field from the `fuzz` log span in invariant
  and max-mode campaigns, so the line reads `fuzz{threads=N}` instead of
  `fuzz{contract=Name threads=N}`
- Invariant campaigns now report failed assertions discovered while replaying
  the corpus, instead of seeding coverage and ignoring those panics

## [0.9.3] - 2026-08-21

### Added

### Changed

- Upgraded solc dependency to v0.3.2
- ForkDB parse errors now include the raw JSON-RPC response body so provider
  failures like error objects or malformed batches are visible in fuzzer logs
- Automatic `.env` loading uses the current working directory instead of the
  project directory

### Fixed

## [0.9.2] - 2026-08-14

### Added

- `rvm.fork` resolves Flare-family network hardforks (Durango → Shanghai, Etna
  → Cancun, pre-Durango → London) from go-flare's upgrade schedule instead of
  defaulting to the newest spec
- `--max-failures N` to collect up to N distinct failed assertions (invariant
  mode only) before stopping the campaign, with each one shrunk and reported
  separately
- `max_*` harness functions: read-only, no-argument `uint256` getters where
  reverted or empty results score `0` and any value above `0` is the finding
- Trace decoding falls back to common standard events (ERC20
  `Transfer`/`Approval`, ERC721 `ApprovalForAll`, WETH9 `Deposit`/`Withdrawal`,
  Ownable `OwnershipTransferred`) when no project artifact declares them,
  rendering names and arguments instead of raw `emit Log(0x...)` lines

### Changed

- The fuzzing lifecycle is one `fuzz` tracing span carrying the harness
  contract and thread count, with consistent `started`, `progress`, and
  `finished` events; the campaign log file records the fuzz-phase duration when
  the span closes
- Per-function statistics log `kind function` as the message (e.g.
  `handler deposit calls=60.2K gas=11.14 G reverts=0`) instead of a generic
  `Function statistics` message with a `function` field
- Maxxing campaigns that find no improvement log a `warn` naming the objective
  (e.g.
  `objective=max_profit No sequence improved the max value   (best stayed at 0)`)
  instead of an `error`
- Removed the redundant `Called setup` log line and the `Ripfuzz out. see   ya`
  farewell line
- Fuzzing progress lines now log structured `key=value` fields (matching the
  final campaign summary) instead of `·`-separated prose
- Terminal log lines print a simple local `HH:MM:SS` timestamp without the
  module target; the campaign log file keeps the full RFC 3339 timestamp with
  target
- Trace output hangs children, call context, logs, storage, and result lines
  directly under each frame's name (aligned regardless of gas amount),
  replacing the `--- Call #N ---` header with a `[N]` counter on the root frame
  line
- `--fail-on-revert` is replaced by `--stop-on-revert`: any reverted
  transaction stops the campaign (invariant and maxxing mode), writes the full
  trace to `fulltrace.log`, dumps a compact trace (call context and storage
  changes omitted) to the log and stderr, names both file paths in the error,
  and exits with a failure instead of shrinking
- A failed `setup()` after a successful deployment stops the campaign like
  `--stop-on-revert`: full trace to `fulltrace.log`, compact trace to log and
  stderr, both paths named in the error
- A failed harness deployment stops the campaign like `--stop-on-revert`: full
  trace to `fulltrace.log`, compact trace to log and stderr, both paths named
  in the error
- Failed-assertion and max-value findings now dump their traces like
  `--stop-on-revert` without failing the campaign: full trace to
  `fulltrace.log` (per-finding `fulltrace-N.log` or `fulltrace-max-N.log`),
  compact trace to log and stderr, both paths named
- Maxxing campaigns no longer track failed assertions or enter the shrinker on
  a revert
- Upgraded solc dependency to v0.1.1
- Fuzzer and shrinker progress logs one compact line every 3 seconds, with the
  full statistics printed after the phase finishes
- Terminal status output now goes through `tracing`; `--disable-log` disables
  all log output (terminal and campaign log file)
- Campaign mode is selected automatically: a harness with a `max_*` function
  runs in max mode, which supports exactly one max function and rejects
  `invariant_*` functions
- Renamed the maxxing campaign type from `MaxCampaign` to `MaxxingCampaign`.
- Fuzzer types now live under `fuzzers`.
- Shrinker types now live under `shrinkers`: `Shrinker` is renamed to
  `InvariantShrinker` and `MaxShrinker` to `MaxxingShrinker`; the `max` module
  was removed.
- Campaign dispatch moved into `commands::run::run`; `CampaignKind::Max` is
  renamed to `CampaignKind::Maxxing`.

### Fixed

- Fork RPC batches that mix cached and missing keys no longer kill the fetcher
  thread with `fetcher did not receive all keys`, which stalled campaigns after
  new storage slots appeared
- Campaign worker failures are no longer swallowed: any failed or panicked
  fuzzer/shrinker thread exits the campaign after all workers settle, with the
  full cause chain (e.g.
  `revm transaction failed: database error: RPC rate limited: …`) instead of
  only the outer message
- Skipped build artifacts now warn with the artifact file path and full error
  chain instead of a bare cause message printed twice
- Build artifacts are loaded once per campaign, so trace contexts reuse them
  instead of re-reading the build output directory (which duplicated artifact
  parse errors in the log)
- `--stop-on-revert` traces stop at the first reverted transaction: only the
  calls up to and including it are re-run and dumped
- Mid-transaction `rvm.fork` switches no longer drop or leak remote state
  written earlier in the same transaction (e.g. `rvm.store`/`rvm.deal` on fork
  A then `rvm.fork` to B): journaled remote mutations commit to the active fork
  overlay before the switch, and local harness accounts stay shared across
  forks
- Fork transport JSON-RPC payloads are logged at `debug` instead of `info`, so
  default runs no longer flood the terminal with full payload lines

## [0.9.1] - 2026-08-07

### Added

- `rvm.getEnv` cheatcode to read environment variables as strings:

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
  working directory). Values are available to `rvm.getEnv`. Existing process
  environment variables take precedence over `.env`.

- `rvm.fork` cheatcode to create or select a remote chain fork:

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

  Campaigns always start as an empty sandbox. Call `rvm.fork` in `setup` or
  action modifiers to opt into remote state. Multiple forks are cached and
  selected by `(url, block)`. Local accounts (harness, deployer, `rvm.addr`
  results) persist across switches. Remote state is isolated per fork, so the
  same address on two chains (e.g. a bridge on Ethereum and Polygon) keeps
  independent storage and balances. Coverage is keyed by bytecode hash, not
  address. Single-arg `rvm.fork` defaults: retries 3, backoff 100ms, timeout
  30s, no rate limit (same as the former CLI defaults).

### Changed

- RVM address is now derived from `keccak256("ripfuzz cheatcode")` instead of
  Foundry's `hevm cheat code`:

  ```text
  // before (Foundry HEVM)
  0x7109709ECfa91a80626fF3989D68f67F5b1DD12D

  // after
  0x628dC59F11F72B611132eC40437F125ba1312F08
  ```

  Harnesses must point `rvm` at the new address (ripfuzz-std `Harness` already
  does this).

- `ripfuzz run <HARNESS>` accepts a bare harness name (`Harness`) or a full
  artifact id (`src/Harness.sol:Harness`). When multiple contracts share the
  same name, the command lists the matching full ids to choose from

- Upgraded solc dependency to v0.0.14

- Campaign directory IDs include seconds
  (`.ripfuzz/campaigns/YYYY-MM-DD-HHMMSS-<uuid>/`) so campaigns started in the
  same minute are easier to tell apart

- Fork mode is driven entirely by `rvm.fork` in the harness. CLI flags
  `--rpc-url`, `--rpc-block`, `--rpc-retries`, `--rpc-backoff`,
  `--rpc-timeout`, and `--rpc-rate-limit` are removed. Single-arg
  `rvm.fork(url, block)` uses built-in defaults (retries 3, backoff 100ms,
  timeout 30s, no rate limit). Override via `rvm.fork(url, block, ForkConfig)`

- Removed the library helper `Chain::fork_with_transport`. Tests and campaigns
  create an empty sandbox and opt into remote state with `rvm.fork` only.

- Removed the startup log for spawning the test chain (including empty-sandbox
  chain id, EVM version, block number, and timestamp). Empty vs fork is decided
  at runtime by `rvm.fork`, so those defaults were misleading

### Fixed

- `rvm.fork` now applies the forked block's EVM `SpecId` (and matching mainnet
  gas params) to the active chain config. Previously only the former
  `Chain::fork` path did this, so harnesses that called `rvm.fork` kept the
  empty-sandbox hardfork instead of the remote chain's hardfork at that height
  (opcodes, gas schedule, and blob base-fee fraction).

- Traces now surface calls to empty accounts clearly. A successful call with no
  bytecode shows `← [stop] (no code)`, and a parent empty revert that follows
  such a call decodes as `no contract code at <address>` instead of plain
  `reverted`. This makes `--fail-on-revert` failures actionable when a harness
  hits remote addresses without `rvm.fork`.

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

[unreleased]: https://github.com/pyk/ripfuzz/compare/v0.9.4...HEAD
[0.9.4]: https://github.com/pyk/ripfuzz/compare/v0.9.3...v0.9.4
[0.9.3]: https://github.com/pyk/ripfuzz/compare/v0.9.2...v0.9.3
[0.9.2]: https://github.com/pyk/ripfuzz/compare/v0.9.1...v0.9.2
[0.9.1]: https://github.com/pyk/ripfuzz/compare/v0.9.0...v0.9.1
[0.9.0]: https://github.com/pyk/ripfuzz/releases/tag/v0.9.0
