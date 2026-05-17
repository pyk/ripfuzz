# Chain Abstraction Refactoring Plan

A comprehensive plan to consolidate raptor's scattered revm integration into a
single, composable, functional `Chain` abstraction under `src/chain`.

---

## Executive Summary

Today raptor's EVM integration is split across three loosely coupled modules:

- `src/evm.rs` — deployment, `setUp()`, sequence execution, property checking,
  and error decoding
- `src/inspector.rs` — `CoverageInspector` that mutates an external
  `LocalCoverage` via revm's `Inspector` trait
- `src/trace.rs` — `CallTraceInspector` that also implements `Inspector`,
  handles Foundry `vm.label` cheatcodes, and formats human-readable traces

This scattering makes it hard to answer three questions:

1. Where is the **initial chain state** created?
2. Where is the **post-setup snapshot** stored?
3. How does a **worker** turn a call sequence into coverage?

This plan introduces a `Chain` abstraction that answers all three questions in
one place. The new API is functional:

```rust
let chain = Chain::initialize(artifact)?.setup()?;
let output = chain.execute(&call_sequence)?;
```

`output` contains everything the worker needs: coverage, traces, property
results, and call metadata. The `chain` itself is never mutated by `execute`, so
workers can share a single initialized `Chain` and each sequence starts from an
identical post-setup state.

---

## Motivation

### Current Problems

**1. State ownership is unclear** `EvmRunner::from_target` creates an
`InMemoryDB`, deploys a contract, optionally calls `setUp()`, and then stores
the resulting `deployed_db` inside `EvmRunner`. Every sequence execution clones
this field (`let mut db = self.deployed_db.clone();`), but the clone logic lives
inside `run_sequence`, not in a dedicated state-management layer.

**2. Mutable side-effects leak through arguments** `EvmRunner::run_sequence`
takes `inspector::CoverageInspector<'a>` by value, but the inspector holds
`&'a mut LocalCoverage`. The caller must create a mutable `LocalCoverage`, pass
a mutable reference to the inspector, and then inspect the `LocalCoverage` after
the call. This is indirect: the real "output" of execution is coverage, but
coverage is an input-side mutation.

**3. Trace and coverage inspectors are independent silos** `CoverageInspector`
and `CallTraceInspector` both implement `Inspector<CTX, EthInterpreter>`, yet
they know nothing about each other. There is no `CompositeInspector` that can
run both at once, so today raptor can either trace or collect coverage, but not
both in the same execution without manual wiring.

**4. Extending to Foundry cheatcodes requires touching three files** Adding
`vm.prank` support would require changes in:

- `evm.rs` — to intercept the cheatcode call before it reaches the contract
- `trace.rs` — because cheatcodes are already partially handled there
  (`vm.label`)
- `inspector.rs` — because coverage must skip or account for cheatcode frames

There is no single extension point.

**5. Worker logic is tightly coupled to revm details** `Worker::run` creates an
`EvmRunner`, manages `LocalCoverage`, and handles nonce increments manually.
These are chain-level concerns, not worker-level concerns.

### How Other Fuzzers Solve This

| Fuzzer      | Abstraction                                   | Pattern                                                                                                                                                                                     |
| ----------- | --------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Medusa**  | `TestChain`                                   | Owns blocks, state DB, config, and cheatcode contracts. `TestChain.CallMessage` returns `MessageResults` containing execution result + additional tracer data. State is cloned per message. |
| **Foundry** | `Cheatcodes` inspector + `CheatcodesExecutor` | A single inspector holds all cheatcode state. `CheatcodesExecutor` trait abstracts nested EVM operations so cheatcodes can run sub-calls without leaking revm details.                      |
| **Echidna** | `World`                                       | Immutable world state that is copied for each sequence. Execution produces a `Result` containing coverage and transactions.                                                                 |

Raptor needs an abstraction that combines Medusa's clear state ownership,
Foundry's composable inspector model, and Echidna's functional data flow.

---

## Design Goals

1. **Clear chain initialization** — one function call turns a `ContractArtifact`
   into a deployed, initialized chain.
2. **Clear chain setup** — a separate step runs `setUp()` (if present) and
   snapshots the resulting state.
3. **Clear worker usage** — the worker holds a `Chain` and calls
   `chain.execute(&sequence)`.
4. **No mutation in arguments** — `execute` takes `&self` and an immutable
   sequence, returning an owned `ExecutionOutput` that contains coverage data.
5. **Module organization** — all chain code lives under `src/chain`.
6. **Extensible** — the inspector system must support adding Foundry cheatcodes
   (`vm.prank`, `vm.warp`, etc.) without changing `executor.rs` or
   `worker/mod.rs`.

---

## Architecture Overview

```
src/chain/
├── mod.rs          // Chain struct, public exports
├── init.rs         // Deployment: artifact -> deployed contract address + initial DB
├── setup.rs        // setUp() execution: initial DB -> post-setup DB snapshot
├── state.rs        // ChainState: InMemoryDB + block context + nonce + known contracts
├── executor.rs     // execute(&self, &[Call]) -> ExecutionOutput
├── output.rs       // ExecutionOutput, CallMeta, PropertyResult
├── error.rs        // DeploymentError, SetupError, ExecutionError
├── inspectors/
│   ├── mod.rs      // Inspector composition traits
│   ├── coverage.rs // CoverageInspector (owned, builds LocalCoverage internally)
│   └── trace.rs    // TraceInspector (owned, builds TraceTree)
└── cheatcodes/
    ├── mod.rs      // CheatcodeInspector, cheatcode dispatch table
    └── standard.rs // vm.label, vm.warp, vm.prank implementations
```

### Core Types

```rust
// src/chain/mod.rs
pub struct Chain {
    config: ChainConfig,
    state: ChainState,
    contract_address: Address,
    properties: Vec<([u8; 4], String)>,
    contract_abi: JsonAbi,
}

pub struct ChainConfig {
    pub caller: Address,
    pub gas_limit: u64,
    pub max_sequence_calls: usize,
}

impl Chain {
    /// 1. Compile initcode, deploy, verify deployment success.
    pub fn initialize(artifact: &ContractArtifact) -> Result<Self, ChainInitError>;

    /// 2. Run setUp() if present, snapshot the resulting state.
    pub fn setup(self) -> Result<Self, ChainSetupError>;

    /// 3. Execute a call sequence against a cloned post-setup state.
    pub fn execute(&self, calls: &[Call]) -> Result<ExecutionOutput, ChainExecutionError>;
}
```

```rust
// src/chain/output.rs
pub struct ExecutionOutput {
    pub coverage: LocalCoverage,
    pub trace: Option<TraceTree>,
    pub call_meta: Vec<CallMeta>,
    pub property_results: Vec<PropertyResult>,
    pub all_ok: bool,
}

pub struct PropertyResult {
    pub name: String,
    pub selector: [u8; 4],
    pub passed: bool,
}

pub struct CallMeta {
    pub block_number: u64,
    pub block_timestamp: u64,
}
```

---

## Module-by-Module Design

### 1. `chain/init.rs` — Deployment

Responsibility: turn a `ContractArtifact` into a deployed contract address and
an `InMemoryDB` containing the deployed code.

- Insert `CALLER` account with balance.
- Insert Foundry VM dummy contract (for `extcodesize` checks) — today in
  `trace::insert_foundry_vm`, moved here because it is part of chain
  initialization.
- Build a one-off EVM with `CallTraceInspector` (for deployment traces only) and
  `TxEnv::Create`.
- Run deployment transaction.
- Extract contract address or return a structured `DeploymentError` with trace.
- Return a `ChainState` containing the post-deployment DB.

**Why separate from `setup.rs`:** Some campaigns may want to inspect the chain
before `setUp()` runs (e.g., for debugging constructor-only state). Keeping them
as distinct logical phases makes the lifecycle explicit.

**Reference:** Medusa's `NewTestChain` creates genesis state and then applies
genesis alloc before any messages are sent. Raptor's `init.rs` is equivalent to
"deploy the target as part of genesis".

### 2. `chain/setup.rs` — Setup

Responsibility: call `setUp()` once if the ABI contains it, snapshot the
resulting DB.

- Check ABI for `setUp()` selector (`0x0a9254e4`).
- If present, clone the initial DB from `init.rs`, run `setUp()` as a CALL
  transaction.
- On failure, return `SetupError` with trace.
- On success, store the final DB as `Chain.state`.

**Snapshot semantics:** `ChainState` is an immutable snapshot. `Chain.execute`
will clone it internally. This matches Echidna's `World` copy model and Medusa's
per-message state clone.

### 3. `chain/state.rs` — State Management

Responsibility: encapsulate everything that must be cloned for each sequence
execution.

```rust
pub struct ChainState {
    pub db: InMemoryDB,
    pub block_number: u64,
    pub block_timestamp: u64,
    pub caller_nonce: u64,
    pub known_contracts: HashMap<Address, (String, JsonAbi)>,
}
```

`ChainState` implements `Clone` (derived, since `InMemoryDB` is `Clone`). It
also provides helper methods:

- `advance_block(number_delay: u64, time_delay: u64)` — applies Medusa's delay
  rules (each block gets a unique timestamp, first call in sequence may stay at
  same block).
- `next_nonce(&mut self) -> u64` — increments and returns caller nonce.

**Why a wrapper:** Today `run_sequence` mutates `evm.ctx.block.number`,
`evm.ctx.block.timestamp`, and `nonce` inline. Moving this into `ChainState`
makes the mutation local and testable, and removes block/nonce logic from the
executor loop.

### 4. `chain/executor.rs` — Sequence Execution

Responsibility: the only place that builds a revm context and runs a sequence.

`Chain::execute` takes `&self`, an immutable sequence, and an `ExecutionOptions`
struct so callers can opt into expensive features only when needed.

```rust
pub struct ExecutionOptions {
    /// Enable call-trace collection. Default: `false`.
    /// Tracing is expensive (allocates per frame) so the hot fuzzing loop
    /// leaves it disabled. It is enabled only for crash reproduction or
    /// when the user explicitly requests a trace.
    pub trace: bool,
}

impl Default for ExecutionOptions {
    fn default() -> Self {
        Self { trace: false }
    }
}
```

```rust
impl Chain {
    pub fn execute(
        &self,
        calls: &[Call],
        opts: ExecutionOptions,
    ) -> Result<ExecutionOutput> {
        // 1. Clone the immutable post-setup state.
        let mut state = self.state.clone();

        // 2. Build inspectors (owned, no external mutable references).
        let mut coverage_inspector = inspectors::CoverageInspector::new();
        let mut trace_inspector = opts.trace.then(|| {
            inspectors::TraceInspector::new(self.state.known_contracts.clone())
        });

        // 3. Run each call in the sequence.
        let mut call_meta = Vec::with_capacity(calls.len());
        let mut all_ok = true;

        for (idx, call) in calls.iter().enumerate().take(self.config.max_sequence_calls) {
            state.advance_block(call.block_number_delay, call.block_timestamp_delay);

            let tx = TxEnv {
                caller: self.config.caller,
                kind: TxKind::Call(self.contract_address),
                data: Bytes::from(call.encode()),
                gas_limit: self.config.gas_limit,
                nonce: state.next_nonce(),
                ..Default::default()
            };

            let mut evm = build_evm(
                state.db,
                &coverage_inspector,
                trace_inspector.as_mut(),
            );
            let result = evm.inspect_tx_commit(tx)?;

            call_meta.push(CallMeta {
                block_number: state.block_number,
                block_timestamp: state.block_timestamp,
            });

            if !result.is_success() {
                all_ok = false;
                break;
            }

            // Update state.db from the EVM's committed journal.
            state.db = evm.ctx.journaled_state.database;
        }

        // 4. Check properties (read-only calls against the final state).
        let property_results = self.check_properties(&mut state)?;

        // 5. Assemble output from owned inspectors.
        Ok(ExecutionOutput {
            coverage: coverage_inspector.into_coverage(),
            trace: trace_inspector.map(|t| t.into_trace_tree()),
            call_meta,
            property_results,
            all_ok,
        })
    }
}
```

**Key properties:**

- `&self` is never mutated.
- `calls` is an immutable slice.
- `ExecutionOutput` is fully owned.
- `coverage_inspector` and `trace_inspector` are owned by `execute` and consumed
  to produce the output. No `&mut LocalCoverage` is passed in.
- `trace` is `Option<TraceTree>`: `None` when tracing is disabled (the default),
  `Some` only when the caller opts in via `ExecutionOptions`.

**Reference:** Medusa's `TestChain` uses a `MessageResults` struct that carries
`AdditionalResults map[string]any` for tracers. Raptor's `ExecutionOutput` is
the typed equivalent.

### 5. `chain/inspectors/` — Inspector Composition

Responsibility: collect execution data without leaking mutable references.

**Problem with current design:** `Inspector<CTX, EthInterpreter>` methods take
`&mut self`, which is fine for the trait, but raptor's current
`CoverageInspector` holds `&'a mut LocalCoverage`, forcing the caller to manage
the coverage buffer's lifetime.

**New design:** Each inspector owns its own buffer and provides a consuming
`into_*` method.

```rust
// src/chain/inspectors/coverage.rs
pub struct CoverageInspector {
    local: LocalCoverage,
    current_call_depth: u64,
    current_contract: Option<ContractId>,
    contract_stack: Vec<Option<ContractId>>,
    last_pc: usize,
}

impl CoverageInspector {
    pub fn new() -> Self { ... }
    pub fn into_coverage(self) -> LocalCoverage { self.local }
}

**Intentional design: fresh `LocalCoverage` per execution.**
`CoverageInspector` creates a brand-new `LocalCoverage` on every `Chain::execute`
call and moves it into `ExecutionOutput` via `into_coverage`. There is no
buffer-reuse or pooling. The allocation cost is acceptable because:

- Coverage maps are small (one `Vec<u8>` per contract, typically a few KB).
- The clarity of ownership outweighs the micro-optimization: `ExecutionOutput`
  owns its coverage, no caller has to remember to `clear()` a shared buffer,
  and there is no risk of stale data leaking between sequences.
- If profiling later shows this as a bottleneck, pooling can be added
  transparently inside `CoverageInspector::new()` without changing the API.
```

```rust
// src/chain/inspectors/trace.rs
pub struct TraceInspector {
    stack: Vec<CallNode>,
    roots: Vec<CallNode>,
    initcode_map: HashMap<Bytes, (String, JsonAbi)>,
    address_names: HashMap<Address, String>,
    address_abis: HashMap<Address, JsonAbi>,
}

impl TraceInspector {
    pub fn new(initcode_map: HashMap<Bytes, (String, JsonAbi)>) -> Self { ... }
    pub fn into_trace_tree(self) -> TraceTree { ... }
    pub fn format(&self) -> String { ... }
}
```

**Composition:** revm's `Inspector` trait does not support "inspector of
inspectors" out of the box, but raptor can define a `CompositeInspector` struct:

```rust
pub struct CompositeInspector {
    coverage: CoverageInspector,
    trace: TraceInspector,
    cheatcodes: Option<CheatcodeInspector>,
}

impl<CTX> Inspector<CTX, EthInterpreter> for CompositeInspector {
    fn step(&mut self, interp: &mut Interpreter<EthInterpreter>, ctx: &mut CTX) {
        self.coverage.step(interp, ctx);
        self.trace.step(interp, ctx);
        if let Some(ref mut c) = self.cheatcodes {
            c.step(interp, ctx);
        }
    }
    // ... delegate every Inspector method
}
```

This matches Foundry's pattern: Foundry's `Cheatcodes` is a single inspector,
but it internally delegates to `TracingInspector` and other subsystems via
`CheatcodesExecutor`. Raptor's `CompositeInspector` is simpler because it does
not need to support nested EVM closures for cheatcodes (yet).

### 6. `chain/cheatcodes/` — Dedicated Cheatcode Files (Medusa Scope)

Responsibility: intercept calls to `VM_ADDRESS` and apply stateful cheatcodes.

**Scope boundary:** Raptor supports only the cheatcodes that Medusa supports.
This is a deliberate compatibility decision: Medusa's standard cheatcode set is
battle-tested for fuzzing campaigns and covers the operations most contracts
need (block manipulation, account patching, pranking, assertions, snapshots, and
basic FFI). Foundry has a much larger surface (forking, broadcasting,
serialization, TOML/JSON parsing, Ed25519, etc.) that is oriented toward
scripting and testing workflows, not fuzzing. Raptor will extend beyond this set
only when a fuzzing use case justifies it.

Today `trace.rs` already intercepts `vm.label(address, string)` and silently
drops the call from the trace. This is a cheatcode, but it is implemented inside
the trace inspector. The new design moves every cheatcode into a dedicated file
under `src/chain/cheatcodes/`. Each file is a self-contained module that exports
one or more dispatch functions, making them independently testable and
maintainable.

#### Module Layout

```
src/chain/cheatcodes/
├── mod.rs          // CheatcodeInspector, dispatch table, VM_ADDRESS constant
├── state.rs        // vm.warp, vm.roll, vm.fee, vm.coinbase, vm.difficulty, vm.prevrandao, vm.chainId
├── account.rs      // vm.deal, vm.etch, vm.setNonce, vm.getNonce, vm.load, vm.store
├── prank.rs        // vm.prank, vm.prankHere, vm.startPrank, vm.stopPrank
├── snapshot.rs     // vm.snapshot, vm.revertTo
├── label.rs        // vm.label
├── assert.rs       // vm.assertTrue, vm.assertEq, vm.assertFalse, etc.
├── string.rs       // vm.toString, vm.parseUint, vm.parseAddress, vm.getCode
├── wallet.rs       // vm.addr, vm.sign
└── ffi.rs          // vm.ffi
```

No `mock.rs`, `assume.rs`, `env.rs`, `fork.rs`, `broadcast.rs`, `fs.rs`,
`crypto.rs`, `debug.rs`, or `log.rs` files are needed at this stage because
Medusa does not support those categories. If raptor later adds mocking or
environment cheatcodes, they get their own file under the same directory.

#### Dispatch Pattern

The `CheatcodeInspector` does not implement cheatcode logic inline. Instead it
holds a dispatch table (`HashMap<[u8; 4], CheatcodeFn>`) populated at
construction time. Each cheatcode file registers its selectors:

```rust
// src/chain/cheatcodes/mod.rs
pub type CheatcodeFn = fn(
    &mut CheatcodeState,
    &mut ChainState,
    &Bytes,            // full calldata (selector + abi-encoded args)
) -> Option<CallOutcome>;

pub struct CheatcodeInspector {
    state: CheatcodeState,
    dispatch: HashMap<[u8; 4], CheatcodeFn>,
}

pub struct CheatcodeState {
    pub prank: Option<PrankState>,
    pub start_prank: Option<StartPrankState>,
    pub labels: HashMap<Address, String>,
    pub snapshots: Vec<InMemoryDB>,
    pub ffi_enabled: bool,
}
```

```rust
// src/chain/cheatcodes/prank.rs
use super::{CheatcodeFn, CheatcodeState, ChainState};

pub fn register(dispatch: &mut HashMap<[u8; 4], CheatcodeFn>) {
    dispatch.insert(PRANK_SELECTOR, handle_prank);
    dispatch.insert(PRANK_HERE_SELECTOR, handle_prank_here);
    dispatch.insert(START_PRANK_SELECTOR, handle_start_prank);
    dispatch.insert(STOP_PRANK_SELECTOR, handle_stop_prank);
}

fn handle_prank(
    state: &mut CheatcodeState,
    _chain: &mut ChainState,
    input: &Bytes,
) -> Option<CallOutcome> {
    // decode new_caller from input[4..]
    // set state.prank for single-call use
}
```

```rust
// src/chain/cheatcodes/mod.rs — constructor
impl CheatcodeInspector {
    pub fn new() -> Self {
        let mut dispatch = HashMap::new();
        state::register(&mut dispatch);
        account::register(&mut dispatch);
        prank::register(&mut dispatch);
        snapshot::register(&mut dispatch);
        label::register(&mut dispatch);
        assert::register(&mut dispatch);
        string::register(&mut dispatch);
        wallet::register(&mut dispatch);
        ffi::register(&mut dispatch);

        Self {
            state: CheatcodeState::default(),
            dispatch,
        }
    }
}
```

#### Why Per-File Cheatcodes Are Better

1. **Testability** — `prank.rs` can have its own `#[cfg(test)]` module that
   creates a `CheatcodeState`, feeds it raw bytes, and asserts on the resulting
   `PrankState`. No EVM needed.
2. **Discoverability** — contributors know exactly which file to open when
   adding or fixing a cheatcode.
3. **Reviewability** — a PR that adds `vm.mockCall` only touches `mock.rs` and
   `mod.rs` (for the registration line).
4. **Scope clarity** — because the file list maps directly to Medusa's supported
   categories, it is immediately obvious what is in scope and what is not.

#### Complete Cheatcode Inventory (Medusa-Compatible)

The following table lists every cheatcode raptor should support, grouped by
category. The selector column is the 4-byte keccak256 selector. The "Implemented
in" column maps to the dedicated file above. Only Medusa's standard cheatcodes
are listed; Foundry-only extensions are excluded.

**State / Block manipulation**

| Selector     | Name         | Signature                     | File       |
| ------------ | ------------ | ----------------------------- | ---------- |
| `0xe17bd987` | `warp`       | `warp(uint256)`               | `state.rs` |
| `0x1f7b4d48` | `roll`       | `roll(uint256)`               | `state.rs` |
| `0xb5b8b202` | `fee`        | `fee(uint256)`                | `state.rs` |
| `0xbc60e744` | `coinbase`   | `coinbase(address)`           | `state.rs` |
| `0x9b441214` | `difficulty` | `difficulty(uint256)` (no-op) | `state.rs` |
| `0x2ba5d5f2` | `prevrandao` | `prevrandao(bytes32)`         | `state.rs` |
| `0x8d8f8a3d` | `chainId`    | `chainId(uint256)`            | `state.rs` |

**Account manipulation**

| Selector     | Name       | Signature                                  | File         |
| ------------ | ---------- | ------------------------------------------ | ------------ |
| `0x1407c37c` | `deal`     | `deal(address, uint256)`                   | `account.rs` |
| `0xb5d88c03` | `etch`     | `etch(address, bytes)`                     | `account.rs` |
| `0x1774d3b5` | `setNonce` | `setNonce(address, uint64)`                | `account.rs` |
| `0x2f391c2c` | `getNonce` | `getNonce(address)` returns `uint64`       | `account.rs` |
| `0x4d2301cc` | `load`     | `load(address, bytes32)` returns `bytes32` | `account.rs` |
| `0x52ef6b2c` | `store`    | `store(address, bytes32, bytes32)`         | `account.rs` |

**Labeling**

| Selector     | Name    | Signature                | File       |
| ------------ | ------- | ------------------------ | ---------- |
| `0xc657c718` | `label` | `label(address, string)` | `label.rs` |

**Prank**

| Selector     | Name         | Signature             | File       |
| ------------ | ------------ | --------------------- | ---------- |
| `0xca669fa7` | `prank`      | `prank(address)`      | `prank.rs` |
| `0x2b8dac2d` | `prankHere`  | `prankHere(address)`  | `prank.rs` |
| `0x45f57d02` | `startPrank` | `startPrank(address)` | `prank.rs` |
| `0xde00347e` | `stopPrank`  | `stopPrank()`         | `prank.rs` |

**Snapshot**

| Selector     | Name       | Signature                          | File          |
| ------------ | ---------- | ---------------------------------- | ------------- |
| `0xb5610ece` | `snapshot` | `snapshot()` returns `uint256`     | `snapshot.rs` |
| `0xb308e46f` | `revertTo` | `revertTo(uint256)` returns `bool` | `snapshot.rs` |

**Wallet / Crypto**

| Selector     | Name   | Signature                         | File        |
| ------------ | ------ | --------------------------------- | ----------- |
| `0xf863551f` | `addr` | `addr(uint256)` returns `address` | `wallet.rs` |
| `0x1600fc3e` | `sign` | `sign(uint256, bytes32)`          | `wallet.rs` |

**String / Type conversion**

| Selector     | Name           | Signature                                | File        |
| ------------ | -------------- | ---------------------------------------- | ----------- |
| `0x2fbe31fa` | `toString`     | `toString(address)` returns `string`     | `string.rs` |
| `0x4f0cb259` | `toString`     | `toString(bool)` returns `string`        | `string.rs` |
| `0xbe680e08` | `toString`     | `toString(uint256)` returns `string`     | `string.rs` |
| `0x65d29ccf` | `toString`     | `toString(int256)` returns `string`      | `string.rs` |
| `0x3b533f4a` | `toString`     | `toString(bytes32)` returns `string`     | `string.rs` |
| `0x4f49367b` | `toString`     | `toString(bytes)` returns `string`       | `string.rs` |
| `0x2e33d057` | `parseUint`    | `parseUint(string)` returns `uint256`    | `string.rs` |
| `0x6c4c0f6c` | `parseInt`     | `parseInt(string)` returns `int256`      | `string.rs` |
| `0x9dd3216e` | `parseBool`    | `parseBool(string)` returns `bool`       | `string.rs` |
| `0x72eb5f63` | `parseAddress` | `parseAddress(string)` returns `address` | `string.rs` |
| `0xf0606581` | `parseBytes`   | `parseBytes(string)` returns `bytes`     | `string.rs` |
| `0xd3a91596` | `parseBytes32` | `parseBytes32(string)` returns `bytes32` | `string.rs` |
| `0x98e0c3fe` | `getCode`      | `getCode(string)` returns `bytes`        | `string.rs` |

**Assertions**

| Selector     | Name          | Signature                       | File        |
| ------------ | ------------- | ------------------------------- | ----------- |
| `0xc6817cfd` | `assertTrue`  | `assertTrue(bool)`              | `assert.rs` |
| `0x9711715a` | `assertFalse` | `assertFalse(bool)`             | `assert.rs` |
| `0x5d995caa` | `assertEq`    | `assertEq(bool, bool)`          | `assert.rs` |
| `0x60b28ab7` | `assertEq`    | `assertEq(uint256, uint256)`    | `assert.rs` |
| `0xe5ffc81e` | `assertEq`    | `assertEq(int256, int256)`      | `assert.rs` |
| `0xd5fada32` | `assertEq`    | `assertEq(address, address)`    | `assert.rs` |
| `0x6c9a2a4a` | `assertEq`    | `assertEq(bytes32, bytes32)`    | `assert.rs` |
| `0x0b34d8fc` | `assertEq`    | `assertEq(string, string)`      | `assert.rs` |
| `0xa1b0b503` | `assertEq`    | `assertEq(bytes, bytes)`        | `assert.rs` |
| `0x981b24d0` | `assertNotEq` | `assertNotEq(bool, bool)`       | `assert.rs` |
| `0x3e5e0e13` | `assertNotEq` | `assertNotEq(uint256, uint256)` | `assert.rs` |
| `0x273b6912` | `assertNotEq` | `assertNotEq(int256, int256)`   | `assert.rs` |
| `0x9a6a4c0b` | `assertNotEq` | `assertNotEq(address, address)` | `assert.rs` |
| `0x2f4f5cc8` | `assertNotEq` | `assertNotEq(bytes32, bytes32)` | `assert.rs` |
| `0x3f2f62f7` | `assertNotEq` | `assertNotEq(string, string)`   | `assert.rs` |
| `0x6d12f6bc` | `assertNotEq` | `assertNotEq(bytes, bytes)`     | `assert.rs` |
| `0x1010e834` | `assertLt`    | `assertLt(uint256, uint256)`    | `assert.rs` |
| `0xe01867c9` | `assertLt`    | `assertLt(int256, int256)`      | `assert.rs` |
| `0x1c4e41f8` | `assertLe`    | `assertLe(uint256, uint256)`    | `assert.rs` |
| `0x3e0a4244` | `assertLe`    | `assertLe(int256, int256)`      | `assert.rs` |
| `0x1c4efa98` | `assertGt`    | `assertGt(uint256, uint256)`    | `assert.rs` |
| `0xb433d668` | `assertGt`    | `assertGt(int256, int256)`      | `assert.rs` |
| `0x3e0be2f5` | `assertGe`    | `assertGe(uint256, uint256)`    | `assert.rs` |
| `0x322b1d42` | `assertGe`    | `assertGe(int256, int256)`      | `assert.rs` |

**FFI**

| Selector     | Name  | Signature                       | File     |
| ------------ | ----- | ------------------------------- | -------- |
| `0x0a94d92e` | `ffi` | `ffi(string[])` returns `bytes` | `ffi.rs` |

**Not in scope (Foundry-only, no Medusa equivalent)**

- `assume`, `assumeNoRevert` — Medusa uses these for symbolic execution, not for
  fuzzing campaigns.
- `mockCall`, `mockCallRevert` — not in Medusa's standard set.
- `envOr`, `setEnv`, `envAddress`, etc. — environment variable cheatcodes are
  Foundry test-suite features.
- `createFork`, `selectFork`, `rollFork` — fork testing is Foundry-specific.
- `broadcast`, `startBroadcast` — deployment scripting is Foundry-specific.
- `readFile`, `writeFile`, `projectRoot`, `exists`, etc. — filesystem cheatcodes
  are Foundry-specific.
- `signEd25519`, `verifyEd25519`, `createEd25519Key` — advanced crypto is
  Foundry-specific.
- `breakpoint`, `pauseGasMetering`, `record`, `accesses`,
  `startStateDiffRecording` — debugging cheatcodes are Foundry-specific.
- `recordLogs`, `getRecordedLogs` — log recording is Foundry-specific.
- `snapshotState`, `revertToState`, `snapshotGasLastCall` — Medusa only has
  `snapshot` / `revertTo`.

If raptor later decides to support any of these, they get a new file under
`src/chain/cheatcodes/` and a single registration line in `mod.rs`.

**Key design rules for every cheatcode file:**

1. **Pure decode + state mutation** — The handler receives raw bytes, decodes
   arguments with `alloy_dyn_abi`, mutates `CheatcodeState` or `ChainState`, and
   returns an optional `CallOutcome` (for calls that short-circuit the EVM).
2. **No revm imports in cheatcode files** — Cheatcode handlers work at the
   `Bytes` / `ChainState` level. Only `mod.rs` imports revm types to build the
   `CallOutcome` that the `CompositeInspector` needs.
3. **Unit-testable without EVM** — Every handler is a standalone `fn` that can
   be unit-tested by constructing a `CheatcodeState`, a `ChainState`, and a
   `Bytes` payload.
4. **Selector constants in each file** — Each file defines `const` selectors
   locally, exported only for tests.
5. **ChainState exposes helpers for DB mutation** — Cheatcodes that modify
   account state (e.g. `vm.deal`, `vm.etch`, `vm.store`) mutate `ChainState.db`
   directly through thin helper methods on `ChainState` such as
   `set_balance(addr, value)`, `set_code(addr, bytes)`,
   `set_storage(addr, slot, value)`. This keeps cheatcode files free of revm
   internals while making it trivial to add new state-mutating cheatcodes:
   implement the helper once on `ChainState`, then call it from the handler.

**Reference:** Medusa's `standard_cheat_code_contract.go` registers cheatcodes
as methods on a precompile contract. Raptor mirrors Medusa's exact selector set
and semantics, but uses a Rust dispatch-table pattern (rather than Go closures)
because it fits revm's `Inspector` trait architecture naturally.

---

## Worker Integration

Today `Worker::run` does this:

```rust
let runner = evm::EvmRunner::from_target(&self.artifact)?;
let mut local_coverage = LocalCoverage::new();
// loop:
    local_coverage.clear();
    let inspector = inspector::CoverageInspector::new(&mut local_coverage);
    let result = runner.run_sequence(&calls, inspector)?;
```

After the refactor, `Worker::run` does this:

```rust
let chain = Chain::initialize(&self.artifact)?.setup()?;
// loop:
    let output = chain.execute(&calls)?;
    let local_coverage = output.coverage;
    let all_ok = output.all_ok;
    let property_triggered = output.property_results.iter().any(|p| !p.passed);
```

**Benefits:**

- `Worker` no longer knows what an `InMemoryDB` is.
- `Worker` no longer manages `LocalCoverage` clearing or inspector lifetimes.
- `Worker` can optionally capture traces for failed sequences by reading
  `output.trace`.
- `Worker` can be extended to support cheatcodes without changing
  `Chain::execute` if cheatcodes are enabled via `ChainConfig`.

**Thread safety:** `Chain.execute(&self, ...)` only reads from `self` and clones
`self.state` at the start. Therefore a single `Chain` can be shared across
workers via `Arc<Chain>`, eliminating per-worker chain initialization cost. Each
worker calls `chain.execute(...)` concurrently; the internal `InMemoryDB` clone
guarantees isolation.

---

## Data Flow Summary

```
ContractArtifact
       |
       v
  Chain::initialize()
       |
       v
  Chain::setup()
       |
       v
   Chain { state: ChainState, ... }
       |
       |   Arc<Chain> shared across workers
       v
  Worker thread N
       |
       v
  chain.execute(&[Call])
       |
       +-- clones ChainState
       +-- builds owned inspectors
       +-- runs revm sequence
       +-- checks properties
       |
       v
  ExecutionOutput {
      coverage: LocalCoverage,
      trace: TraceTree,
      property_results: Vec<PropertyResult>,
      call_meta: Vec<CallMeta>,
      all_ok: bool,
  }
```

No mutable references cross the `Chain -> Worker` or `Worker -> Chain` boundary.
The only mutation happens inside `execute`, on the cloned `ChainState` and owned
inspectors, which are dropped when the function returns.

---

## Glossary Alignment

This plan uses raptor's canonical terms (from `docs/glossary.md`):

- **Target contract** — the Solidity contract passed to `Chain::initialize`.
- **Setup function** — `setUp()` executed inside `Chain::setup`.
- **Function call** — each `Call` in the sequence passed to `Chain::execute`.
- **Property function** — checked inside `Chain::execute` after the sequence,
  producing `PropertyResult`.
- **Worker** — holds `Chain` (or `Arc<Chain>`) and calls `execute` in a loop.
- **Campaign** — orchestrates chain initialization and worker threads.

---

## Migration Roadmap

### Phase 1 — Scaffold `src/chain` (no deletions)

1. Create `src/chain/mod.rs` with `Chain`, `ChainConfig`, placeholder methods.
2. Create `src/chain/init.rs`, copy deployment logic from `evm.rs`.
3. Create `src/chain/setup.rs`, copy `setUp()` logic from `evm.rs`.
4. Create `src/chain/state.rs`, define `ChainState`.
5. Create `src/chain/output.rs`, define `ExecutionOutput`, `CallMeta`,
   `PropertyResult`.
6. Create `src/chain/error.rs`, define typed errors.
7. Add `pub mod chain;` to `src/lib.rs`.

**Verification target:** `cargo check` passes with both old and new modules
present.

### Phase 2 — Inspectors

1. Create `src/chain/inspectors/mod.rs` with `CompositeInspector` trait helpers.
2. Create `src/chain/inspectors/coverage.rs`, migrate `CoverageInspector` to
   owned-buffer design.
3. Create `src/chain/inspectors/trace.rs`, migrate `CallTraceInspector`.
4. Write unit tests for each inspector in isolation.

**Verification target:** `cargo test -- chain::inspectors` passes.

### Phase 3 — Executor

1. Implement `Chain::execute` in `src/chain/executor.rs`.
2. Wire `CompositeInspector` inside `execute`.
3. Implement property checking as a post-sequence loop inside `execute`.

**Verification target:** A test deploys a target, calls `Chain::execute` with a
sequence, and asserts on `ExecutionOutput.coverage` and
`ExecutionOutput.all_ok`.

### Phase 4 — Worker Cutover

1. Change `Worker::run` to use `Chain` instead of `EvmRunner` +
   `CoverageInspector`.
2. Remove `LocalCoverage::clear()` call from worker (not needed; each
   `ExecutionOutput` has a fresh `LocalCoverage`).
3. Update `PropertyFailure` construction to read from
   `ExecutionOutput.property_results`.

**Verification target:** `cargo test` campaign tests pass (e.g.,
`catches_l1_simple_knob_dragon`).

### Phase 5 — Cleanup

1. Remove `src/evm.rs`, `src/inspector.rs`, `src/trace.rs`.
2. Remove `pub mod evm;`, `pub mod inspector;`, `pub mod trace;` from
   `src/lib.rs`.
3. Update any remaining references (e.g., `worker/mod.rs` tests that import
   `evm::CallMeta` should import `chain::CallMeta`).

**Verification target:** `make check` and `make test` pass cleanly.

### Phase 6 — Cheatcode Extension Point (Medusa-Compatible)

1. Create `src/chain/cheatcodes/mod.rs` with `CheatcodeInspector`,
   `CheatcodeState`, `CheatcodeFn`, and the registration pattern.
2. Create `src/chain/cheatcodes/label.rs` — move `vm.label` from
   `TraceInspector` into a dedicated file. Write unit tests that verify decoding
   and state mutation without an EVM.
3. Create `src/chain/cheatcodes/state.rs` — implement `vm.warp`, `vm.roll`,
   `vm.fee`, `vm.coinbase`, `vm.difficulty` (no-op), `vm.prevrandao`,
   `vm.chainId`.
4. Create `src/chain/cheatcodes/account.rs` — implement `vm.deal`, `vm.etch`,
   `vm.setNonce`, `vm.getNonce`, `vm.load`, `vm.store`.
5. Create `src/chain/cheatcodes/prank.rs` — implement `vm.prank`,
   `vm.prankHere`, `vm.startPrank`, `vm.stopPrank`.
6. Create `src/chain/cheatcodes/snapshot.rs` — implement `vm.snapshot`,
   `vm.revertTo`.
7. Create `src/chain/cheatcodes/assert.rs` — implement `vm.assertTrue`,
   `vm.assertFalse`, `vm.assertEq` (all overloads), `vm.assertNotEq`,
   `vm.assertGt`, `vm.assertGe`, `vm.assertLt`, `vm.assertLe`.
8. Create `src/chain/cheatcodes/string.rs` — implement `vm.toString` (all
   overloads), `vm.parseUint`, `vm.parseInt`, `vm.parseBool`, `vm.parseAddress`,
   `vm.parseBytes`, `vm.parseBytes32`, `vm.getCode`.
9. Create `src/chain/cheatcodes/wallet.rs` — implement `vm.addr`, `vm.sign`.
10. Create `src/chain/cheatcodes/ffi.rs` — implement `vm.ffi` (gated behind a
    config flag for security).
11. Wire `CheatcodeInspector` into `CompositeInspector` inside `executor.rs`.
12. Enable cheatcodes via `ChainConfig` (default: enabled for fuzzing).

**Verification targets:**

- Unit tests in each `cheatcodes/*.rs` file pass without starting an EVM.
- Integration test: a fixture contract uses `vm.warp`, `vm.label`, and
  `vm.startPrank` in `setUp()`; the trace output shows correct labels and block
  timestamps.
- Integration test: a fixture contract uses `vm.snapshot` and `vm.revertTo`;
  state correctly rolls back after revert.
- Medusa compatibility: a contract that runs under Medusa's cheatcode set should
  produce identical behavior under raptor (same labels, same prank semantics,
  same snapshot IDs).

---

## Resolved Decisions

The following questions were raised during early drafting and are now resolved.
The answers are baked into the design above.

1. **Tracing cost in hot loops — RESOLVED.** `Chain::execute` accepts
   `ExecutionOptions { trace: bool }`. Tracing is **disabled by default** so the
   hot fuzzing loop never pays the allocation cost. It is enabled only for crash
   reproduction or when the user explicitly requests a trace. See
   `chain/executor.rs` above for the `ExecutionOptions` struct and the
   `Option<TraceInspector>` pattern.

2. **Coverage map sizing — RESOLVED.** **Fresh `LocalCoverage` per execution is
   the intended design.** `CoverageInspector` creates a brand-new
   `LocalCoverage` on every `Chain::execute` call and moves it into
   `ExecutionOutput` via `into_coverage`. There is no buffer-reuse or pooling.
   The clarity of ownership outweighs the allocation cost: no caller has to
   remember to `clear()` a shared buffer, and there is zero risk of stale data
   leaking between sequences. If profiling later shows this as a bottleneck,
   pooling can be added transparently inside `CoverageInspector::new()` without
   changing the API. See `chain/inspectors/coverage.rs` above.

3. **Nonce management / cheatcode state mutation — RESOLVED.** **Cheatcodes
   mutate `ChainState.db` directly through helper methods on `ChainState`.**
   `ChainState` exposes thin wrappers such as `set_balance(addr, value)`,
   `set_code(addr, bytes)`, `set_storage(addr, slot, value)` so cheatcode
   handlers do not need revm imports or deep DB knowledge. Adding a new
   state-mutating cheatcode means: (a) adding a helper on `ChainState` if one
   does not exist, and (b) calling it from the handler. See `chain/cheatcodes/`
   design rule #5 above.
