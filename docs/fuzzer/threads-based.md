# Thread-Based Fuzzing Plan

A plan to replace LibAFL with a thread-based fuzzing engine.

## Motivation

LibAFL provides coverage-guided fuzzing primitives, but raptor uses them in a
way that introduces friction:

- **One `unsafe` block** is required for `StdMapObserver::from_mut_ptr` on a
  shared coverage map.
- **Process-based parallelism** via `LlmpRestartingEventManager` spawns child
  processes, requires shared-memory allocation, broker ports, and a
  `send_exiting` + sleep dance to avoid broker hangs.
- **Heavy dependency surface**: `libafl` + `libafl_bolts` pull in many crates
  for features raptor does not need (crash reproduction, fork server, AFL++
  compatibility, etc.).
- **Result aggregation through temp files**: workers write JSON to `/tmp` and
  the main process scans the directory by prefix after the launcher exits. This
  is fragile.

Switching to thread-based fuzzing lets us:

1. Eliminate all `unsafe` code.
2. Replace process spawning with `std::thread::spawn`.
3. Replace shared memory with `Arc<RwLock<_>>` and `mpsc` channels.
4. Remove `libafl` and `libafl_bolts` from `Cargo.toml`.
5. Keep every existing mutator, seed generator, EVM runner, and corpus type
   unchanged in behavior.

## Design Goals

- **Corpus management across workers/threads should be similar to how Medusa,
  Echidna, and Foundry manage it.**
- **Corpus** is a collection of corpus items.
- **Corpus Item** is a sequence of function calls.
- **Coverage-guided**: a sequence is added to the corpus only when it increases
  total known coverage.
- **Weighted random selection**: sequences used for mutation are chosen with a
  probability proportional to their weight, so newer / more interesting
  sequences are picked more often.
- **Deterministic seeds**: each worker gets a distinct seed offset so runs are
  reproducible when the base seed is fixed.

## Corpus Architecture (Medusa-style)

### In-Memory Representation

```rust
pub struct Corpus {
    /// Coverage-increasing sequences available for mutation.
    pub items: Vec<CorpusItem>,

    /// Property failures discovered during the campaign (not used for mutation).
    pub failures: Vec<CorpusItem>,

    /// Sequences loaded from disk that have not been replayed yet.
    pub pending: Vec<CorpusItem>,

    /// Weighted random chooser for picking a mutation target.
    chooser: WeightedRandomChooser,

    /// Global coverage map across all known sequences.
    coverage: CoverageMap,

    /// Directory for persistent storage, if any.
    storage_dir: Option<PathBuf>,
}

pub struct CorpusItem {
    pub calls: Vec<Call>,
    pub weight: u64,
}
```

### Thread Safety

The `Corpus` lives behind an `Arc<RwLock<Corpus>>` shared by all workers:

- **Read lock**: `random_item_for_mutation()` copies a `CorpusItem` out.
- **Write lock**: `check_and_update_coverage()` merges a local coverage map and,
  if new edges were found, appends the item.
- **Write lock**: `add_failure()` appends a failure item.

Because a corpus write is rare (only when new coverage is found), contention is
low. Most worker iterations only need a read lock for mutation target selection.

### Coverage Map

Coverage is stored **per-contract** so that every hit can be mapped back to
source code. See `docs/fuzzer/source-mapping.md` for the full design.

```rust
pub struct CoverageMap {
    pub contracts: HashMap<ContractId, ContractCoverage>,
}

pub struct ContractCoverage {
    pub contract_id: ContractId,
    /// Bytecode-level hit map: PC -> AFL bucket.
    pub edges: Vec<u8>,
    /// Call-depth sensitivity for hot PCs (first 1,024).
    pub depths: Vec<u64>,
    /// Revert tracking (bit-packed).
    pub reverts: Vec<u64>,
}
```

- **Per-worker**: each worker owns a `LocalCoverage` with the same shape. The
  `CoverageInspector` writes into it during execution, keyed by the current
  contract's bytecode hash.
- **Global**: the `Corpus` holds the `CoverageMap` with the accumulated state.
- **Merge**: the worker merges its local map into the global per-contract maps.
  A sequence is "interesting" if any of the following are true:
    1. **New edge** — a PC was hit for the first time.
    2. **New feature** — a PC's AFL hitcount bucket increased.
    3. **New depth** — a PC (in the first 1,024) was hit at a new call depth.
    4. **New revert** — a PC was hit in a reverted call for the first time.

This combines Foundry's AFL-style bucketing, Echidna's call-depth sensitivity,
and Medusa's revert tracking.

### Key Methods

| Method                                                                       | Purpose                                                          |
| ---------------------------------------------------------------------------- | ---------------------------------------------------------------- |
| `random_item_for_mutation() -> Option<CorpusItem>`                           | Weighted random pick, cloned. Returns `None` if corpus is empty. |
| `check_and_update_coverage(local: &LocalCoverage, item: CorpusItem) -> bool` | Merge local map into global. If interesting, add item to corpus. |
| `add_failure(item: CorpusItem)`                                              | Add a failure item (not used for mutation).                      |
| `pop_pending_item() -> Option<CorpusItem>`                                   | Replay persisted corpus at startup; each item returned once.     |
| `flush_to_disk()` / `load_from_disk()`                                       | JSON persistence, same format as today.                          |

### Corpus Item Deduplication

A `CorpusItem` is added only if its call sequence is not already present. The
`Call` type already derives `PartialEq`, so `Vec<Call>` equality compares every
byte of every selector and argument buffer. The check is an O(n) scan across
`items`, `failures`, and `pending`. This is the same approach Medusa uses and is
fast enough for typical corpus sizes (hundreds to thousands of entries).

## Mutator Decoupling

Today every mutator implements
`libafl::mutators::Mutator<corpus::CallSequenceInput, S>` where `S: HasRand`. We
replace this with a simple trait that operates directly on the call vector:

```rust
pub trait Mutator {
    fn mutate(&mut self, rng: &mut impl Rng, calls: &mut Vec<Call>) -> MutationResult;
}

pub enum MutationResult {
    Mutated,
    Skipped,
}
```

All nine mutators are updated to use `&mut impl Rng` instead of `HasRand`, and
return the plain enum instead of `libafl::mutators::MutationResult`. The actual
mutation logic stays identical.

### Mutator List (unchanged behavior)

1. `SequenceSwapMutator`
2. `SequenceInsertMutator`
3. `SequenceDeleteMutator`
4. `SequenceSpliceMutator`
5. `SequenceInterleaveMutator`
6. `SequenceHeadMutator`
7. `SequenceTailMutator`
8. `SequenceArgMutator`
9. `SequenceDelayMutator`

## Worker Thread

Each worker is a `std::thread` running this loop:

```
for i in 0..local_max_runs:
    if let Some(item) = corpus.pop_pending_item() {
        // Startup phase: replay persisted corpus without mutation
        execute(item, is_new=false)
    } else {
        // Normal fuzzing phase
        let mut item = if rng.bool() && corpus.has_entries() {
            let base = corpus.random_item_for_mutation();
            apply_random_mutator(base)
        } else {
            generate_random_sequence()
        };
        execute(item, is_new=true)
    }
```

`execute(item, is_new)`:

1. Clone `deployed_db` into a fresh EVM.
2. Create a `LocalCoverage` sized to the target contract's runtime bytecode.
3. Run the calls with a `CoverageInspector` that writes into the local coverage
   map, keyed by the current contract's bytecode hash.
4. If the sequence reverts early, skip coverage / property checks.
5. If all calls succeed, check properties.
6. If a property returns `true`, send `PropertyFailure` through the `mpsc`
   channel.
7. If new coverage (new edge, feature, depth, or revert), send the item to the
   corpus (via write lock).
8. If this was a replayed corpus item and it executed cleanly, mark it valid for
   mutation.

No `unsafe`, no shared memory, no process spawning.

## Campaign Orchestration

The `Campaign::run()` method:

1. Build the `Arc<RwLock<Corpus>>`.
2. Spawn `workers` threads, each with:
    - A clone of `ContractArtifact`
    - A clone of `CampaignConfig`
    - A copy of `selectors`
    - A base `seed + worker_id`
    - A clone of the `Corpus` `Arc`
    - A clone of the `mpsc::Sender<PropertyFailure>`
3. Each worker runs its loop and then returns a `WorkerResult` via `JoinHandle`.
4. The main thread drains the failure channel and joins handles.
5. Aggregate total runs and failures into `CampaignResult`.

This removes:

- `LlmpRestartingEventManager`
- `ShMemProvider`
- `Launcher`
- Temp-file JSON aggregation
- `send_exiting` + sleep hacks
- Core-affinity mask parsing

## What Stays Unchanged

| Module                   | Notes                                                                                                                       |
| ------------------------ | --------------------------------------------------------------------------------------------------------------------------- |
| `src/corpus/call.rs`     | `Call` struct unchanged.                                                                                                    |
| `src/corpus/item.rs`     | New module for `CorpusItem`. `CallSequenceInput` is removed entirely.                                                       |
| `src/evm.rs`             | `EvmRunner` and `SequenceResult` unchanged.                                                                                 |
| `src/inspector.rs`       | Rewritten to use per-contract `LocalCoverage` instead of flat `&mut [u8]`. Contract identity tracked per call frame.        |
| `src/contract/`          | Artifact building and ABI parsing unchanged. `ContractArtifact` gains `init_source_map` and `runtime_source_map` fields.    |
| `src/trace.rs`           | Foundry-style traces unchanged.                                                                                             |
| `src/campaign/seeds.rs`  | Seed generation unchanged.                                                                                                  |
| `src/campaign/config.rs` | Add `worker_count()`, keep other fields. Remove `broker_port`, add optional `corpus_dir`.                                   |
| `src/commands/fuzz.rs`   | CLI unchanged except default workers (use `std::thread::available_parallelism()` instead of `libafl_bolts::core_affinity`). |

## Dependency Changes

**Remove** from `Cargo.toml`:

- `libafl`
- `libafl_bolts`

**Add** (if not already present):

- `fastrand` (or `rand`) — lightweight RNG for workers and mutators.

## Implementation Order

1. **Parse source maps** — extend `src/foundry/artifact.rs` to extract and parse
   `sourceMap` strings from Foundry artifacts. Add `SourceMap` types to
   `src/contract/artifact.rs`.
2. **Build `Corpus`** — create `src/corpus/corpus.rs` with thread-safe corpus +
   per-contract coverage logic, tests.
3. **Rewrite `CoverageInspector`** — update `src/inspector.rs` to use
   `LocalCoverage` keyed by contract bytecode hash instead of a flat byte slice.
4. **Decouple mutators** — rewrite the `Mutator` trait in
   `src/worker/mutators/mod.rs`, update all nine mutator files and their tests.
5. **Rewrite `Worker`** — replace `src/worker/mod.rs` with the thread-based
   loop, remove unsafe.
6. **Rewrite `Campaign`** — replace `src/campaign/mod.rs` with
   `std::thread::spawn`, remove LibAFL imports, remove temp file logic.
7. **Clean up `Cargo.toml`** — remove `libafl` + `libafl_bolts`, fix CLI
   defaults.
8. **Run full test suite** — `make test`.

## Open Decisions

1. **RNG crate**: `fastrand` (lightweight, no std features) or `rand` (more
   capabilities, already widely used). Both work; `fastrand` is smaller.
2. **Coverage merge strategy**: AFL-style hitcount buckets (8 levels: 1, 2, 4,
   8, 16, 32, 64, 128). A "new edge" is the first hit (0 -> non-zero). A "new
   feature" is a bucket increase. See `docs/fuzzer/coverage-comparison.md`.
3. **Corpus persistence format**: Keep JSON pretty-printed files
   (`{ts}-{uuid}.json`) so existing corpuses remain readable.
4. **Failure collection**: Use `mpsc::channel` so workers stream failures
   without lock contention, or collect into a shared `Mutex<Vec>` at the end.
   `mpsc` is preferred.
