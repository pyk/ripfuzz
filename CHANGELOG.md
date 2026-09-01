# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a
Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Tester challenges under `fixtures/tester/challenges`: the easy
  `GatedByLiterals` harness gates one failed assertion behind every literal
  kind (`bool`, `uint256`, `uint128`, `int256`, `int8`, `bytes32`, `bytes1`,
  `address`, `bytes`, `string`, and the `1 ether` subdenomination);
  `tests/tester/challenges.rs` asserts the literals are extracted from the
  compiled harness and that the campaign finds all gated assertions within the
  easy budget (run with `make tester-challenges`)
- Tester campaigns seed argument generation with the harness literals: the
  corpus extracts literals from the solc output via the new `Corpus::new()`
  builder (`with_root`, `with_dir`, `with_harness`, `with_handlers`,
  `with_solc_output`), and calls draw from the extracted pools for `uint`,
  `int`, fixed bytes, `address`, `bytes`, and `string` arguments, so gates
  behind constant comparisons are reachable
- `ripfuzz test <harness>` runs a test harness campaign that finds failed
  assertions: it compiles the harness via solc (default contract name from the
  file stem, or `path/File.sol:Name` to pick one), deploys it on a sandbox
  chain, runs the optional `setup` function, fuzzes for Solidity `assert`
  panics (`Panic(0x01)`), shrinks every finding's sequence, prints the logs
  emitted on the way to each failure, and saves execution traces under
  `.ripfuzz/traces`; validation rejects a constructor, `setup`, `summary`, or
  `invariant_*` with arguments or `payable`
- `ripfuzz test` checks `invariant_*` functions after every handler call on a
  throwaway state clone, so invariant state changes are never committed and
  invariants never consume `--max-calls`; findings are deduplicated by
  panicking function and revert output, and only `assert` panics count, so
  `require` and custom-error reverts stay plain control flow
- `ripfuzz test` runs coverage-guided evolutionary fuzzing over a standalone
  corpus (`TestHarness`, `Fuzzer`, `Corpus`, `Shrinker` under `src/tester`),
  with `--threads`, `--max-runs`, `--max-calls`, `--timeout`, `--max-failures`,
  and `--corpus-dir` flags mirroring `ripfuzz max`
- `ripfuzz exec <script>` runs a Solidity script contract: it compiles the
  script (default contract name from the file stem, or `path/File.sol:Name` to
  pick one), deploys it on a sandbox chain, runs the optional `setup` function,
  executes the mandatory `exec` function once, prints emitted logs to the
  console, and saves the execution trace under `.ripfuzz/traces`; validation
  rejects an `exec`/`setup` with arguments or `payable`, and a constructor with
  arguments or `payable`
- `ripfuzz max` runs the optional harness `summary` function on the final
  campaign state after saving the corpus, prints its log output to the console,
  and saves the full execution trace under `.ripfuzz/traces` for offline
  analysis
- `ripfuzz max --log-level` controls log verbosity (default `info`); `debug`
  traces each pending call's handler, success, gas, and revert data, which is
  how the yscrvUSD campaign was debugged
- `ripfuzz max` measures the initial value by calling the harness `value`
  function after deployment and setup, and logs it as the campaign baseline
  (profit is measured against it during maximization); a reverting `value` call
  fails with a dumped execution trace, mirroring deployment and setup
- `ripfuzz max` runs coverage-guided evolutionary fuzzing after the initial
  value is measured: fuzzers draw from a shared corpus of interesting sequences
  (new coverage or a new best value), mutate them via insert, delete, replace,
  duplicate, splice, and argument regeneration, and merge execution coverage
  into a shared map; `--threads` sets the fuzzer count, `--max-runs` bounds the
  total number of sequences, `--max-calls` bounds the sequence length,
  `--timeout` and `--target-value` stop fuzzing early, and progress is logged
  every 3 seconds
- `ripfuzz max` shrinks the best sequence in parallel: shrinkers delete random
  chunks of calls and accept a candidate only when a clean-state replay keeps
  the final value at or above the best value found, so the reported sequence is
  the shortest one found within the budget
- `ripfuzz init` writes a starter `ripfuzz.toml` with `solc = "0.8.36"` in the
  current directory and refuses to overwrite an existing file
- `MaxHarness` validates a compiled `Harness` against the max harness rules (a
  `view`/`pure` `value` function returning `uint256`, no `invariant_*`
  functions, optional `setup` and `summary` functions) and resolves those
  functions for later steps; `ripfuzz max` rejects invalid harnesses before
  deployment
- `ripfuzz max --root <path>` resolves the config file, harness path, and
  output directory relative to the given project root instead of the current
  working directory
- Solc compilation resolves imports through `{root}/remappings.txt`, so
  harnesses importing dependencies via remappings (e.g. `ripfuzz/Harness.sol`)
  compile
- `ripfuzz max --corpus-dir <path>` dumps the corpus of interesting sequences
  when the campaign finishes (and best-effort when it fails), one line per
  entry with its value, new coverage, call count, and sequence; the dump
  defaults to `{root}/.ripfuzz/corpus/{source-file}/{contract}/corpus.log`, so
  a surprising campaign can be analyzed offline
- `ripfuzz max --quiet` (`-q`) suppresses terminal logs by writing the
  subscriber to a null sink, so harnesses forking in tests cannot leak output;
  the deployed address still prints to stdout
- `Vault` and `VaultWithNoise` max challenges cover the approve, deposit, and
  redeem pattern with 28 handlers and fork-free simplified accounting, catching
  future regressions

### Changed

- Renamed the `src/test` module to `src/tester`, moved the test fixtures under
  `fixtures/tester` (`harness-deployment`, `harness-validation`), and the
  integration tests under `tests/tester` (`harness_deployment.rs`,
  `harness_validation.rs`)
- Updated the CLI description to `An extremely fast Smart contract fuzzer.` and
  renamed the `max` command help text to `Find maximum value`
- `ripfuzz.toml` moves the solc settings under the `[solc]` section with
  `version` required and `out` (default `.ripfuzz/solc`), `evm_version`
  (default `prague`), `optimizer` (default `false`), `optimizer_runs` (default
  `200`), `via_ir` (default `false`), and `remappings` (default `[]`); the
  legacy flat `solc = "0.8.36"` form and the top-level `out` field are rejected
  with a config parse error instead of being silently accepted, and
  `ripfuzz init` writes the new shape with the optimizer enabled for 200 runs
- Configured solc remappings resolve imports next to `{root}/remappings.txt`,
  with config entries winning when both map the same prefix
- Upgraded solc dependency to v0.3.5, and renamed the standard JSON input
  `viaIr` key to the `viaIR` key solc expects so `via_ir` compiles through the
  IR-based pipeline
- `ripfuzz max` seeds the shared coverage map with the execution coverage of
  the harness deployment and setup calls, so fuzzers only count edges beyond
  harness initialization as new and corpus entries are not inflated with
  baseline edges
- `ripfuzz max` runs the harness `setup` function after deployment and fails
  with a dumped execution trace when it reverts
- `Solc::compile` returns `SolcOutput` (the resolved `HarnessId` plus the raw
  `StandardJSONOutput`) instead of `Harness`, so callers can extract the target
  contract and build trace contexts from the same compilation result
- `MaxHarness::try_from` accepts `&SolcOutput` and extracts the target contract
  directly from the solc output; the generic `Harness` type in
  `ripfuzz::harness` is removed while `HarnessId` stays for CLI parsing
- `TraceContext` converts from a solc compilation result via
  `From<&SolcOutput>`, building ABI, bytecode, AST, and storage layout entries
  without a Foundry project; `ripfuzz max` dumps the failed deployment trace to
  an absolute `.ripfuzz/traces/<unix-timestamp>-<id>.log` path and logs it
- `ripfuzz max` now deploys the compiled harness on a sandbox chain after
  compilation and prints the deployed address instead of the solc version
- `HarnessId` moved from `ripfuzz::cli` to `ripfuzz::harness`, next to the
  compiled `Harness` type it identifies
- Solc artifacts are written under a namespace derived from the target source
  path (e.g. `.ripfuzz/out/src/Harness.sol/out.json`), so targets sharing an
  out directory never overwrite each other's artifacts; the combined output
  file is renamed from `output.json` to `out.json`
- Upgraded solc dependency to v0.3.4, which re-exports the standard JSON output
  types (`ContractOutput`, `SourceOutput`, `Bytecode`, and friends) at the
  crate root
- Maxxing reports use a consistent score vocabulary: `raw_score` is the value
  returned by a `max_*` call, `base_score` is the raw score observed after
  `setup()`, and `best_score` is the best raw score observed so far. Log fields
  `value=` and `baseline=` become `best_score=` and `base_score=` on the
  progress, finished, and setup lines
- `ripfuzz max` wires the same fork defaults as the campaign command, so
  harness forks share the `.ripfuzz/cache` rpc cache across commands and use a
  conservative batch rate limit that keeps default campaigns under
  public-provider quotas
- `ripfuzz max` persists the corpus as JSON and loads it at startup, so a new
  campaign starts mutating from the previous run's sequences; loaded sequences
  are replayed against the current harness to seed the shared coverage map and
  re-measure entry values, and entries whose value call no longer succeeds are
  dropped; the shrunk best sequence joins the corpus before saving, so the next
  campaign starts from the shortest sequence that reaches the best value;
  entries store each call as its handler signature plus full calldata and are
  re-resolved against the harness ABI on load, with unresolvable entries
  skipped; the file is
  `{root}/.ripfuzz/corpus/{source-file}/{contract}/corpus.json` (or
  `--corpus-dir`), replacing the write-only `corpus.log` dump

### Fixed

- `ripfuzz max` now generates arguments for handlers that take struct (tuple)
  parameters, including arrays and nested structs, by resolving JSON-ABI
  parameters with their components instead of parsing the bare `tuple` type
  string, which crashed the campaign at startup on such harnesses

- `ripfuzz` now loads `{cwd}/.env` at startup for every command instead of only
  `ripfuzz run`, so harnesses using `vm.getEnv` see the same values in `run`
  and `max`; existing environment variables still take precedence

- Max fuzzers now spend half of their mutated sequences extending the current
  best sequence instead of only corpus entries and fresh random sequences, so a
  value that needs a long chain of calls still climbs when decoy handlers
  dilute the corpus; previously a full corpus rejected new-best sequences that
  brought no new coverage, so the best rung was never a mutation base and the
  climb stalled

- Max best tracking now prefers the same value with fewer calls, so mutations
  can free call slots occupied by calls that do not affect the value and extend
  the value further within the call limit

- Corpus now uses AFL-style energy for `pick_item` (finds boost energy even for
  existing ids via `bump_entry`, energy decays only after a mutation that adds
  nothing) and caps at 1024 items evicting lowest-energy entries while never
  evicting the current best, min and max

- Maxxing now treats `max_*` as raw `uint256` score with baseline after
  `setup()` and keeps prefixes that set a new raw max or min, so lossy prefixes
  that are on the path to profit stay in the corpus

- Coverage now records `CALL` targets as `new_jump_edges` via
  `(caller_pc, callee_address)` hash, so calls to different addresses at the
  same PC are distinct even when bytecode is shared

- Coverage is now keyed by `(address, codehash)` for runtime contracts, so the
  first `CALL` into each clone is considered interesting even when bytecode is
  shared; initcode remains keyed by hash

- Fork-mode progress and finished logs now include `rpc_hit`, `rpc_miss`, and
  `rpc_wait`, and loading a fork cache logs `loaded fork cache` with the entry
  count, so a stuck campaign can be diagnosed as RPC-bound vs EVM-bound

- Campaign progress names the current hotspot handler (`hot`, `hot_elapsed`,
  `hot_rpc_miss`), and finished per-function rows include wall time and RPC
  counters, so a slow handler like an unbounded `getQuote` shows up while the
  run is still stuck

- `-q`/`--quiet` suppresses terminal logs while still writing the campaign log
  file

- Fork-mode campaigns now throttle RPC batches to a conservative default of 10
  batches per second so default runs stay under public-provider rate quotas;
  override per fork with `vm.fork(..., ForkConfig{rateLimit: N})` or disable
  with `rateLimit: 0`

- The RPC batch retry loop now logs a warning with the retry number, total
  retries, backoff duration, batch size, endpoint, request payload, and error
  as structured fields; the terminal prints a one-line form (origin-only URL,
  no payload, short error) while the campaign log file keeps the full fields

- Per-thread fuzzer `run` spans now include `fuzzer_id`, so nested logs such as
  RPC retry warnings identify which fuzzer emitted them

- Integration tests pass `--quiet` so `make test` no longer prints campaign
  logs

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
