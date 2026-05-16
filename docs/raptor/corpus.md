# Raptor Corpus

This document describes the production-ready corpus system for raptor. The
corpus is a persistent, human-readable archive of **action sequences** that
drive coverage-guided mutation and preserve **crashes** across fuzzing
**campaigns**.

Raptor builds on LibAFL's built-in on-disk corpus primitives rather than
reinventing storage, serialization, or cross-process mechanics. The plan
synthesizes best practices from Medusa, Echidna, and Foundry.

---

## 1. Overview

A raptor campaign stores two kinds of artifacts on disk:

1. **Coverage action sequences** — sequences that discovered new code coverage.
   These are used as mutation seeds.
2. **Crash action sequences** — sequences that caused a **property** (invariant)
   to return `false`. These are kept for reproducibility but are **not** used
   for mutation.

Both kinds are serialized to **pretty-printed JSON** so engineers can read,
edit, and commit them to version control. When raptor starts a new campaign, it
loads every existing file, replays each sequence to verify it still executes
against the current contract ABI, and rebuilds in-memory coverage maps.

---

## 2. Corpus Format

### 2.1 Directory Layout

If `campaign.corpus_directory` is set (via `raptor.toml` or CLI), raptor expects
the following layout:

```
<corpus_dir>/
├── coverage/
│   ├── worker0/
│   │   ├── 1716123456-00000000-0000-0000-0000-000000000000.json
│   │   └── ...
│   ├── worker1/
│   └── ...
└── crashes/
    ├── worker0/
    │   ├── 1716123456-00000000-0000-0000-0000-000000000001.json
    │   └── ...
    ├── worker1/
    └── ...
```

- **`coverage/`** — per-worker subdirectories for sequences that increased
  coverage. Each worker writes only into its own subdirectory to avoid lock
  contention.
- **`crashes/`** — per-worker subdirectories for sequences that triggered a
  property failure.

If `corpus_dir` is unset, the corpus operates entirely in-memory and never
touches disk.

### 2.2 File Naming Convention

Every file name follows Foundry's proven pattern:

```
<timestamp>-<uuid>.json
```

- **`<timestamp>`** — Unix timestamp in seconds when the entry was written.
- **`<uuid>`** — UUIDv4 that uniquely identifies the corpus entry. Parsed on
  reload so the same in-memory identity can be restored.

This ordering makes directory listings chronological and lexicographically
sortable.

### 2.3 JSON Schema

Each `.json` file is a **pretty-printed JSON array** of `Action` objects. A
single file represents one `CallSequenceInput` (the LibAFL `Input` type).

#### `Action`

```json
[
    {
        "method_name": "deposit",
        "method_signature": "deposit(uint256)",
        "selector": "0xd0e30db0",
        "input_values": ["1234567890000000000"],
        "args": "0x000000000000000000000000000000000000000000000000112210f47de98100",
        "block_number_delay": 1,
        "block_timestamp_delay": 1
    },
    {
        "method_name": "withdraw",
        "method_signature": "withdraw(uint256)",
        "selector": "0x2e1a7d4d",
        "input_values": ["1000"],
        "args": "0x00000000000000000000000000000000000000000000000000000000000003e8",
        "block_number_delay": 0,
        "block_timestamp_delay": 0
    }
]
```

| Field                   | Type       | Description                                                                                                                                                        |
| ----------------------- | ---------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `method_name`           | string     | **Required.** Human-readable function name. Empty string when the selector is unknown.                                                                             |
| `method_signature`      | string     | **Required.** Full function prototype (e.g. `transfer(address,uint256)`). Used to recompute the selector at load time and survive ABI changes. Empty when unknown. |
| `selector`              | hex string | 4-byte function selector. Source of truth for execution.                                                                                                           |
| `input_values`          | array      | **Required.** JSON-friendly representation of each ABI argument (strings, numbers, booleans, nested arrays/structs). Empty when the ABI is unknown.                |
| `args`                  | hex string | Raw ABI-encoded calldata for this call (selector excluded). Source of truth when `input_values` cannot be resolved.                                                |
| `block_number_delay`    | uint64     | How many blocks to advance before this call.                                                                                                                       |
| `block_timestamp_delay` | uint64     | How many seconds to advance before this call.                                                                                                                      |

#### Serialization Rules

1. **Source of truth** — At execution time, raptor uses `selector` + `args`. The
   human-readable fields (`method_name`, `method_signature`, `input_values`) are
   for review and portability only.
2. **Load-time resolution** — When a corpus file is loaded, raptor recomputes
   the 4-byte selector from `method_signature` and looks it up in the target
   contract ABI. If the signature matches, it also validates that `args` is
   consistent with `input_values`. If the signature no longer exists, the action
   falls back to raw `selector` + `args` execution (warning emitted).
3. **Pretty printing** — Every file is written with `serde_json::to_vec_pretty`
   so `git diff` remains readable.

---

## 3. Input Type Extensions

Raptor's LibAFL `Input` type is `CallSequenceInput`. To satisfy the JSON
requirement we extend it rather than replacing it.

### 3.1 Enriched `Call`

```rust
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Call {
    pub selector: [u8; 4],
    pub args: Vec<u8>,
    pub block_number_delay: u64,
    pub block_timestamp_delay: u64,

    // Human-readable fields are required so every JSON file is fully
    // self-describing. When the ABI is unavailable they are empty.
    pub method_name: String,
    pub method_signature: String,
    pub input_values: Vec<serde_json::Value>,
}
```

- Mutators continue to operate on `selector` and `args`. The human-readable
  fields are updated by mutators that have access to the ABI (e.g.,
  `SequenceArgMutator`) so the on-disk JSON is always readable.
- When a call is created for a selector not present in the ABI (e.g. random
  seeding), the fields are initialized to empty strings and an empty array, so
  every file is valid and uniform.

### 3.2 Custom `Input` Persistence

The default LibAFL `Input::to_file` uses Postcard (binary). We override it on
`CallSequenceInput` to emit pretty JSON:

```rust
impl Input for CallSequenceInput {
    fn to_file<P>(&self, path: P) -> Result<(), libafl_bolts::Error>
    where
        P: AsRef<std::path::Path>,
    {
        let bytes = serde_json::to_vec_pretty(self)
            .map_err(|e| libafl_bolts::Error::serialize(format!("json failed: {e}")))?;
        libafl_bolts::fs::write_file_atomic(path, &bytes)
    }

    fn from_file<P>(path: P) -> Result<Self, libafl_bolts::Error>
    where
        P: AsRef<std::path::Path>,
    {
        let mut file = std::fs::File::open(path)?;
        let mut bytes = vec![];
        file.read_to_end(&mut bytes)?;
        let input = serde_json::from_slice(&bytes)
            .map_err(|e| libafl_bolts::Error::serialize(format!("json parse failed: {e}")))?;
        Ok(input)
    }

    fn generate_name(&self, id: Option<libafl::corpus::CorpusId>) -> String {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let uuid = uuid::Uuid::new_v4();
        format!("{ts}-{uuid}.json")
    }
}
```

This satisfies the requirement to use LibAFL built-ins: `InMemoryOnDiskCorpus`
and `OnDiskCorpus` call `Input::to_file` automatically; we only change the
serialization format.

---

## 4. LibAFL Corpus Types

Raptor uses three LibAFL corpus types, each chosen for a specific role:

| Role                             | LibAFL Type                               | Why                                                                                                                 |
| -------------------------------- | ----------------------------------------- | ------------------------------------------------------------------------------------------------------------------- |
| Coverage corpus (mutation seeds) | `InMemoryOnDiskCorpus<CallSequenceInput>` | Keeps every seed in memory for fast mutation selection **and** persists to disk so seeds survive restarts.          |
| Crash corpus (property failures) | `OnDiskCorpus<CallSequenceInput>`         | Stores failures on disk for reproducibility. Does **not** need to reside in memory because crashes are not mutated. |
| In-flight objectives             | `InMemoryCorpus<CallSequenceInput>`       | Temporary staging inside `StdState` before promotion to the on-disk crash corpus.                                   |

### 4.1 State Type Alias Update

```rust
pub(crate) type MyCorpus = InMemoryOnDiskCorpus<CallSequenceInput>;
pub(crate) type MyObjectiveCorpus = OnDiskCorpus<CallSequenceInput>;
pub(crate) type MyState = StdState<MyCorpus, CallSequenceInput, StdRand, MyObjectiveCorpus>;
```

`StdState` is LibAFL's standard state container. By swapping `InMemoryCorpus`
for `InMemoryOnDiskCorpus` as the main corpus, persistence is automatic.

### 4.2 Metadata Format

LibAFL writes `.metadata` sidecars for each `Testcase`. Raptor configures the
corpus with `OnDiskMetadataFormat::JsonPretty` so metadata (execution count,
parent id, objective reason) is also human-readable.

---

## 5. Initialization & Loading

Initialization happens in two phases: **directory setup** (before the EVM is
ready) and **runtime replay** (after deployment and `setUp()`).

### 5.1 Directory Setup (`CampaignBuilder`)

1. If `corpus_dir` is configured, create the layout:
    ```
    <corpus_dir>/coverage/worker<id>/
    <corpus_dir>/crashes/worker<id>/
    ```
2. Instantiate
   `InMemoryOnDiskCorpus::with_meta_format(coverage_dir, Some(JsonPretty))`.
3. Instantiate `OnDiskCorpus::with_meta_format(crash_dir, Some(JsonPretty))`.

If the directory does not exist or is empty, the corpus starts blank.

### 5.2 Pre-Fuzzing Replay

After deployment and `setUp()`, each worker replays its assigned initial corpus.
The campaign distributes the load by scanning all `coverage/` subdirectories and
splitting files across workers (similar to Echidna's chunking).

Replay logic:

1. **Load** each JSON file with `CallSequenceInput::from_file`.
2. **Bind** every action (resolve against the current ABI):
    - Look up `selector` in the target contract ABI.
    - If `method_signature` is present, verify it hashes to the same selector.
    - If the signature no longer exists, the action is **unresolvable**.
3. **Execute** the full sequence on a cloned EVM with coverage collection.
4. **Outcome**:
    - **Valid** — binding and execution succeed. The sequence is added to the
      worker's `InMemoryOnDiskCorpus` so it can be used as a mutation seed.
    - **Invalid** — binding fails (ABI mismatch, method removed) or execution
      errors (revert, out of gas). The sequence is **silently consumed and
      discarded**; it is **not** added to the mutation pool and **not** deleted
      from disk. This follows Medusa's `bindCorpusElement` pattern: replay
      failures are logged at debug level but the on-disk file is preserved so
      `raptor corpus clean` can later report it explicitly.

Only after every initial sequence has been processed does the worker enter the
main mutation loop.

### 5.3 Crash Replay Verification

Crash sequences from `crashes/` are also replayed during initialization,
following the same load → bind → execute flow as coverage sequences. The goal
is to verify that old reproducers still trigger the property failure.

- **Still crashing** — the sequence is kept in the `OnDiskCorpus`.
- **No longer crashing** (bug was fixed, or ABI changed so the sequence is
  invalid) — the sequence is **silently skipped**. It is not removed from disk;
  `raptor corpus clean` handles that.

This mirrors Medusa's behavior: test-result sequences are enqueued alongside
coverage sequences during `Corpus.Initialize` and are subject to the same
`bindCorpusElement` validation.

---

## 6. Corpus Update

### 6.1 Coverage-Driven Addition

Raptor uses LibAFL's `MaxMapFeedback` over a shared edge-coverage map. After
`fuzz_one` executes an input, LibAFL's feedback system decides whether the input
is **interesting** (new edges hit).

If interesting:

1. `StdFuzzer` adds the `CallSequenceInput` to `state.corpus()`.
2. `InMemoryOnDiskCorpus::add` stores the `Testcase` in memory **and** calls
   `save_testcase`, which invokes our custom `Input::to_file` to write pretty
   JSON to the worker's `coverage/` directory.
3. LibAFL's `LlmpRestartingEventManager` broadcasts a `NewTestcase` event to the
   broker. Other workers receive the input and add it to their own local
   `InMemoryOnDiskCorpus`, so every worker eventually benefits from new
   coverage.

### 6.2 Deduplication

- **In-memory** — LibAFL's `StdState` deduplicates by `Input::hash` before
  adding to the corpus.
- **On-disk** — Because we use `Uuid`-based filenames derived from wall-clock
  time, exact duplicate inputs may write multiple files. This is acceptable for
  correctness and simplicity; a post-campaign or periodic deduplication pass
  (see Pruning) cleans them up.

### 6.3 Crash Sequences

When a property returns `false`, the harness returns `ExitKind::Crash`. LibAFL's
`CrashFeedback` marks the input as an objective.

1. `StdFuzzer` adds the input to `state.solutions()` (`OnDiskCorpus`).
2. `OnDiskCorpus::add` writes the JSON to the worker's `crashes/` directory.
3. The crash is broadcast via events so the campaign manager can aggregate
   failures immediately.

Crash entries are **never** added to the coverage corpus and are **not** used
for mutation.

---

## 7. Cross-Worker Sync

Raptor runs workers as separate processes via LibAFL's `Launcher`. The corpus
must propagate interesting sequences across all workers without custom IPC.

### 7.1 How LibAFL Helps

LibAFL's LLMP event protocol already handles this:

- Worker A discovers new coverage → `Event::NewTestcase(input)` is sent to the
  broker.
- Worker B receives the event → `state.add_corpus(Testcase::new(input))` →
  `InMemoryOnDiskCorpus::add` writes JSON to Worker B's `coverage/workerB/`
  directory.

No extra sync logic is required.

### 7.2 Post-Campaign Merge (Optional)

After the campaign finishes, a lightweight merge step can deduplicate coverage
entries across all worker subdirectories into a single canonical corpus:

```
raptor corpus merge <corpus_dir>
```

This command:

1. Scans every `coverage/worker*/` file.
2. Replays each sequence and computes its hash.
3. Keeps only the first occurrence of each unique hash.
4. Writes the merged set into `coverage/merged/`.

This is analogous to Medusa's `medusa corpus clean` and Foundry's implicit
per-worker aggregation.

---

## 8. Pruning & Minimization

Over time the coverage corpus accumulates redundant sequences that collectively
hit the same edges. Raptor implements two maintenance operations, inspired by
Medusa's `PruneSequences` and Echidna's set-based dedup.

### 8.1 Corpus Prune (`raptor corpus prune`)

A greedy minimization pass:

1. Snapshot the current in-memory corpus entries.
2. Create a blank temporary coverage map.
3. Iterate the snapshot in random order.
4. Replay each sequence on a fresh cloned state.
5. If the sequence **does not** add new edges to the temp map, delete its
   on-disk file and disable the in-memory `Testcase`.
6. Repeat until the corpus is a minimal hitting set for the current total
   coverage.

This preserves total coverage while shrinking the seed pool, which speeds up
mutation selection.

### 8.2 Corpus Clean (`raptor corpus clean`)

An explicit validation pass for ABI drift, modeled on Medusa's
`CleanInvalidSequences`. When contracts are refactored or recompiled, old
corpus files may contain stale selectors or signatures. This command scans the
corpus, validates every file, and deletes the ones that can no longer execute.

Command behavior:

1. Create a fresh EVM runner from the current target artifact.
2. Scan every file in `coverage/` and `crashes/`.
3. For each file:
    a. Load the JSON sequence.
    b. **Bind** each action — resolve `method_signature` against the current ABI.
       If the signature is unknown, or the selector no longer matches, the
       sequence is flagged invalid.
    c. **Execute** the full sequence on a cloned EVM state.
       If execution errors (revert, out of gas, panic), the sequence is flagged
       invalid.
    d. Revert the EVM to the base state before testing the next file.
4. **Delete** every invalid file from disk.
5. Report statistics:
   ```
   Total sequences:  150
   Valid sequences:    142
   Invalid sequences:  8
   ```

Unlike the soft skip during pre-fuzzing replay (Section 5.2), `corpus clean`
is a **hard purge** — invalid files are permanently removed. This is run
explicitly by the user after refactoring contracts or before committing the
corpus to version control.

### 8.3 Favored Entry Tracking (Future)

Following Foundry's model, raptor can track per-entry statistics:

- `total_mutations` — how many times this sequence was selected as a mutation
  base.
- `new_finds_produced` — how many of those mutations discovered new coverage.

An entry becomes **favored** when:

```
new_finds_produced / total_mutations > 0.3
```

Favored entries are protected from eviction during in-memory trimming. LibAFL's
`CorpusPowerTestcaseScore` scheduler supports weighting entries by performance
metadata; raptor can attach a custom `FavoredMetadata` to `Testcase` and use it
in a custom scheduler.

---

## 9. Configuration

Corpus behavior is controlled via `CampaignConfig` and `raptor.toml`.

### 9.1 `CampaignConfig` Fields

```rust
pub struct CampaignConfig {
    // ... existing fields ...

    /// Path to the corpus root directory. If set, coverage-guided
    /// persistence is enabled.
    pub corpus_dir: Option<PathBuf>,

    /// Maximum number of action sequences to keep in memory before
    /// evicting unfavored entries. `0` means unlimited.
    pub corpus_max_size: usize,

    /// Minimum number of mutations an entry must survive before it
    /// becomes eligible for eviction.
    pub corpus_min_mutations: usize,

    /// Pretty-print JSON metadata sidecars alongside corpus files.
    pub corpus_pretty_meta: bool,
}
```

### 9.2 `raptor.toml` Example

```toml
[fuzzing.corpus]
directory = "fuzz_corpus"
max_size = 10000
min_mutations = 5
pretty_meta = true
```

---

## 10. Implementation Roadmap

### Phase 1 — JSON Persistence (Foundation)

1. Add human-readable fields to `Call`.
2. Implement `Input::to_file` / `from_file` on `CallSequenceInput` with
   `serde_json`.
3. Switch `MyCorpus` to `InMemoryOnDiskCorpus` and `MyObjectiveCorpus` to
   `OnDiskCorpus`.
4. Add `corpus_dir` to `CampaignConfig` and create per-worker subdirectories.
5. Wire `CampaignBuilder` to instantiate on-disk corpora.
6. Verify that `make test` still passes (in-memory fallback when `corpus_dir` is
   `None`).

### Phase 2 — Replay & Validation

1. Implement pre-fuzzing replay: scan `coverage/` files, distribute across
   workers, validate ABI, warm up coverage maps.
2. Add ABI resolution at load time (`method_signature` → selector lookup).
3. Add stale-sequence detection: skip files where the target contract no longer
   contains the method.

### Phase 3 — Maintenance CLI

1. Implement `raptor corpus clean` (validation pass).
2. Implement `raptor corpus prune` (greedy coverage minimization).
3. Implement `raptor corpus merge` (cross-worker deduplication).

### Phase 4 — Advanced Scheduling (Future)

1. Add `FavoredMetadata` to `Testcase`.
2. Track `total_mutations` and `new_finds_produced` per entry.
3. Introduce a custom scheduler (extending LibAFL's `CorpusPowerTestcaseScore`)
   that biases toward newer and favored entries, similar to Echidna's weighted
   `Set` and Foundry's favorability heuristic.

---

## 11. Summary of Key Types

| Type                                      | Role                                                                           |
| ----------------------------------------- | ------------------------------------------------------------------------------ |
| `CallSequenceInput`                       | LibAFL `Input` type; enriched with ABI metadata and custom JSON persistence.   |
| `InMemoryOnDiskCorpus<CallSequenceInput>` | Coverage corpus: in-memory + on-disk, pretty JSON.                             |
| `OnDiskCorpus<CallSequenceInput>`         | Crash corpus: on-disk only, pretty JSON.                                       |
| `StdState`                                | LibAFL state container holding both corpora.                                   |
| `LlmpRestartingEventManager`              | Cross-worker event bus that propagates new coverage and crashes automatically. |
| `MaxMapFeedback`                          | Coverage feedback that drives additions to the coverage corpus.                |
| `CrashFeedback`                           | Objective feedback that drives additions to the crash corpus.                  |

---

## 12. Design Decisions

| Decision                                                  | Rationale                                                                                                                                                                                    |
| --------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Extend LibAFL corpora rather than reimplement**         | `InMemoryOnDiskCorpus` and `OnDiskCorpus` handle locking, serialization, filename generation, and event propagation. Reimplementing would duplicate hundreds of lines of battle-tested code. |
| **Pretty JSON instead of binary**                         | Engineers must be able to `cat` a corpus file, understand the action sequence, and commit it to git. Medusa and Echidna both prove this is practical at scale.                               |
| **Per-worker subdirectories**                             | Avoids file-lock contention across parallel processes. Foundry uses the same pattern.                                                                                                        |
| **Raw `args` + `selector` as source of truth**            | Survives ABI changes (e.g., argument renaming) because the EVM only cares about calldata. `method_signature` and `input_values` are advisory.                                                |
| **Post-campaign merge instead of real-time global dedup** | Simpler and race-free. Global dedup across processes requires a shared database or coordination protocol; LibAFL's event system already keeps each worker's corpus warm.                     |
| **Separate `coverage/` and `crashes/`**                   | Same distinction Medusa makes. Crashes are reproducers, not seeds.                                                                                                                           |
