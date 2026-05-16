# Foundry Corpus

This document describes the coverage-guided corpus system used by Foundry's fuzz
and invariant test executors. The corpus stores transaction sequences that have
produced interesting coverage, which are later mutated to discover new execution
paths.

---

## 1. Corpus Format

### 1.1 Directory Structure

When corpus persistence is enabled (`fuzz.corpus_dir` or
`invariant.corpus_dir`), Foundry creates a directory tree on disk:

```
<corpus_dir>/
├── worker0/                  # Master worker directory
│   ├── corpus/               # Master's local corpus entries
│   │   ├── <uuid>-<timestamp>.json
│   │   └── <uuid>-<timestamp>.json.gz
│   └── sync/                 # Entries exported by other workers
│       └── <uuid>-<timestamp>.json
├── worker1/                  # Worker 1 directory
│   ├── corpus/
│   └── sync/
└── workerN/
    ├── corpus/
    └── sync/
```

Each parallel fuzzing worker gets its own subdirectory. The master worker is
always `worker0`. Every worker maintains:

- **`corpus/`** — its own local corpus entries that produced new coverage during
  its own runs.
- **`sync/`** — a staging area where other workers deposit new findings via hard
  links.

### 1.2 File Naming Convention

Every corpus entry file follows the pattern:

```
<uuid>-<timestamp>.json[.gz]
```

- **`<uuid>`** — A UUIDv4 that uniquely identifies the corpus entry. Parsed from
  the filename when reloading so the same in-memory identity can be restored.
- **`<timestamp>`** — Unix timestamp in seconds when the entry was written. Used
  during sync to filter out already-imported entries.
- **`.gz`** — Optional gzip compression applied when the serialized JSON exceeds
  4 KiB (`GZIP_THRESHOLD`).

### 1.3 JSON Content Format

Each corpus file contains a JSON array of `BasicTxDetails` objects. A single
`BasicTxDetails` represents one fuzz-generated transaction:

```json
[
    {
        "warp": null, // Optional U256: seconds to advance block.timestamp
        "roll": null, // Optional U256: blocks to advance block.number
        "sender": "0x...", // Address that sends the call
        "target": "0x...", // Contract address being called
        "calldata": "0x..." // Transaction data including 4-byte selector
    }
]
```

The array as a whole is a **call sequence** (for invariant tests) or a single
call (for stateless fuzz tests).

### 1.4 Optimization State File

When an invariant function returns `int256`, Foundry enters **optimization
mode**. The best value and the sequence that produced it are persisted
separately at the corpus root:

```
<corpus_dir>/optimization_best.json
```

Structure:

```json
{
    "best_value": "123...",
    "best_sequence": [
        /* array of BasicTxDetails */
    ]
}
```

This file is loaded by the master worker at startup and seeded into the
in-memory corpus so mutations can build on it.

---

## 2. How It Is Initialized

Initialization happens inside `WorkerCorpus::new`, which is called once per
worker at the start of a fuzz or invariant campaign.

### 2.1 Directory Creation

If `corpus_dir` is configured:

1. The worker computes its subdirectory: `<corpus_dir>/worker<id>/`.
2. Creates `corpus/` and `sync/` inside it (non-fatal on failure).

### 2.2 Master Worker Special Handling (id == 0)

Only the master worker loads existing persisted corpus data. All other workers
start with an empty in-memory corpus.

#### 2.2.1 Optimization State Loading

If `optimization_best.json` exists at the corpus root:

- It is deserialized into `OptimizationState`.
- The best sequence is pushed into the in-memory corpus so the mutation engine
  can evolve it.
- If loading fails, a warning is emitted and the campaign starts without a
  persisted seed.

#### 2.2.2 Persisted Corpus Replay

The master iterates over every file in `<corpus_dir>` (not inside
`worker0/corpus/`, but the root). Each file matching the corpus filename pattern
is:

1. Parsed for its UUID and timestamp.
2. Deserialized into a `Vec<BasicTxDetails>`.
3. Replayed call-by-call through a cloned `Executor`.

During replay, each call's coverage is merged into the worker's `history_map`
and `sancov_history_map`. This **warms up** the coverage history so the campaign
does not re-discover already-known edges.

If a transaction cannot be replayed (e.g., the targeted function or contract is
no longer available), the replay failure is counted in `failed_replays`. For
stateless fuzz tests, if the only input for the fuzzed function cannot be
replayed, the entire file is skipped.

For invariant tests, state is committed after each replayed call so the
sequence's cumulative state effects are preserved.

### 2.3 In-Memory Data Structures

Each `WorkerCorpus` holds:

- **`in_memory_corpus: Vec<CorpusEntry>`** — Loaded + newly discovered entries.
- **`history_map: Vec<u8>`** — Binned hitcounts for EVM edge coverage (size =
  65,536).
- **`sancov_history_map: Vec<u8>`** — Binned hitcounts for SanitizerCoverage
  edges (size = 65,536).
- **`metrics: CorpusMetrics`** — Per-worker counters for edges, features, corpus
  count, and favored items.
- **`new_entry_indices: Vec<usize>`** — Indices of entries added since the last
  sync, used for cross-worker export.
- **`last_sync_timestamp: u64`** — Used to avoid re-importing old sync files.
- **`current_mutated: Option<Uuid>`** — Tracks which corpus entry is being
  mutated in the current run.

---

## 3. How It Is Loaded

### 3.1 Directory Scanning (`read_corpus_dir`)

`read_corpus_dir` enumerates a directory and returns an iterator of
`CorpusDirEntry` values. It:

1. Reads the directory with `std::fs::read_dir`.
2. Filters to regular files.
3. Parses the filename with `parse_corpus_filename`, which splits on the last
   `-` to extract `(uuid, timestamp)`.
4. Silently skips files that do not match the expected pattern.

### 3.2 File Deserialization (`CorpusDirEntry::read_tx_seq`)

Each entry knows how to read itself:

- If the path extension ends in `.gz`, it uses
  `foundry_common::fs::read_json_gzip_file` (streams through a `GzDecoder`).
- Otherwise, it uses `foundry_common::fs::read_json_file` (plain JSON via
  `serde_json`).

Both paths deserialize into `Vec<BasicTxDetails>`.

### 3.3 Sync Loading (`load_sync_corpus`)

Before each sync, a worker scans its own `sync/` directory. It imports only
entries whose `timestamp > last_sync_timestamp`. This prevents processing the
same sync file twice. Empty sequences are skipped with a warning.

### 3.4 Calibration (`calibrate`)

After loading sync entries, the worker **calibrates** them:

1. Each imported sequence is replayed through the EVM.
2. Coverage from each call is merged into `history_map` and
   `sancov_history_map`.
3. If the sequence produces **new coverage** for this worker:
    - The file is moved from `sync/` to the worker's `corpus/` directory.
    - A `CorpusEntry` is added to `in_memory_corpus`.
4. If the sequence does **not** produce new coverage:
    - The file is deleted to avoid wasting disk and memory.

This means a corpus entry may exist on one worker but be discarded on another if
it does not improve that worker's local coverage map.

---

## 4. How It Is Updated

### 4.1 Adding New Entries (`process_inputs`)

At the end of every fuzz or invariant run, `process_inputs` is called with:

- The executed call sequence.
- A boolean indicating whether new coverage was discovered.
- An optional optimization tuple `(best_value, best_sequence)`.

#### 4.1.1 Eligibility

A sequence is persisted only if:

- `new_coverage` is `true`, **or**
- `improved_optimization` is `true` (the run achieved a better `int256` value
  than the previous best).

If only optimization improved (no new coverage), the **best prefix sequence**
(the shortest prefix that achieved the best value) is saved instead of the full
run.

#### 4.1.2 Favorability Tracking

If the current run was produced by mutating an existing corpus entry
(`current_mutated` is set), that entry's stats are updated:

- `total_mutations += 1`
- `new_finds_produced += 1` (only if the run was interesting)

An entry becomes **favored** when:

```
new_finds_produced / total_mutations > 0.3
```

Favored entries are protected from eviction. The `CorpusMetrics.favored_items`
counter is adjusted whenever an entry's favorability status changes.

#### 4.1.3 Disk Persistence

For interesting runs:

1. A new `CorpusEntry` is created with a fresh UUID.
2. It is written to `<worker_dir>/corpus/<uuid>-<timestamp>.json[.gz]`.
3. Its index is recorded in `new_entry_indices` for the next sync.
4. It is appended to `in_memory_corpus`.

### 4.2 Mutation Strategies

When coverage-guided fuzzing is active, `new_inputs` (for invariant tests) or
`new_input` (for stateless fuzz tests) generates the next input by mutating the
in-memory corpus.

#### 4.2.1 Sequence Mutations (Invariant Tests)

A random `MutationType` is selected for each run:

| Mutation       | Description                                                                                                            |
| -------------- | ---------------------------------------------------------------------------------------------------------------------- |
| **Splice**     | Takes a slice from a primary corpus entry and appends a slice from a secondary entry.                                  |
| **Repeat**     | Replaces a random range in the sequence with repeated copies of one existing call.                                     |
| **Interleave** | Zips two corpus sequences, randomly picking either primary or secondary call at each position.                         |
| **Prefix**     | Overwrites the prefix of a sequence with newly generated calls.                                                        |
| **Suffix**     | Overwrites the suffix of a sequence with newly generated calls.                                                        |
| **Abi**        | ABI-decodes a random call's calldata, mutates one or more argument values via `mutate_param_value`, and re-encodes it. |

The `current_mutated` field is set to the UUID of the primary corpus entry being
mutated so its favorability stats can be updated after the run.

#### 4.2.2 Call Mutation (Stateless Fuzz Tests)

For stateless fuzz tests, a single call is mutated:

1. If the in-memory corpus is empty, a fresh call is generated from the fuzz
   strategy.
2. Otherwise, a random corpus entry is picked, its first `BasicTxDetails` is
   cloned, and its arguments are ABI-mutated against the target function.

### 4.3 Corpus Eviction (`evict_oldest_corpus`)

To prevent unbounded memory growth, workers evict old entries before generating
new inputs:

- The in-memory corpus must be larger than `corpus_min_size` (default `0`).
- The oldest entry whose `total_mutations > corpus_min_mutations` (default `5`)
  **and** that is **not favored** is removed.
- When an entry is removed, `new_entry_indices` are adjusted (shifted down or
  removed if they pointed to the evicted index).

Eviction does **not** delete the on-disk file; it only frees the in-memory copy.

### 4.4 Cross-Worker Sync Protocol

Foundry runs fuzzing with multiple workers. The corpus must eventually propagate
interesting sequences across all workers. Sync happens periodically (every
`SYNC_INTERVAL` runs, staggered by worker ID to avoid thundering herd).

#### 4.4.1 Non-Master Worker Export (`export_to_master`)

Workers `1..N` export their new entries to the master by creating **hard
links**:

- Source: `<workerN/corpus>/<filename>`
- Target: `<worker0/sync>/<filename>`

Hard links are used so no extra disk space is consumed for duplicates.

#### 4.4.2 Master Import & Distribution (`export_to_workers`)

The master worker:

1. Calibrates entries from its own `sync/` directory (moves interesting ones to
   `worker0/corpus/`, deletes uninteresting ones).
2. Distributes its `corpus/` entries to all other workers' `sync/` directories
   via hard links.
3. Only distributes entries newer than `last_sync_timestamp`.

#### 4.4.3 Metrics Sync (`sync_metrics`)

Each worker computes deltas since its last sync:

- `cumulative_edges_seen`
- `cumulative_features_seen`
- `corpus_count`
- `favored_items`

These deltas are atomically added to a shared `GlobalCorpusMetrics` structure
(using `AtomicUsize` with `Ordering::Relaxed`). The global metrics are reported
in progress bars and JSON pulse events.

### 4.5 Edge Coverage Collection

Coverage is collected per-call and merged into the worker's history maps.

#### 4.5.1 EVM Edge Coverage

During EVM execution, an `EdgeCovInspector` records edge hits into a `Vec<u8>`
coverage map stored on `RawCallResult::edge_coverage`. When
`merge_edge_coverage` is called:

- Hitcounts are binned using the AFL algorithm:
    ```
    1 → 1, 2 → 2, 3 → 4, 4..7 → 8, 8..15 → 16, 16..31 → 32, 32..127 → 64, 128..255 → 128
    ```
- If the binned value exceeds the history map's current value, the history map
  is updated.
- If the history map value was `0`, this counts as a **new edge**.
- If the history map value was non-zero but lower, this counts as a **new
  feature** (new hitcount bin).

#### 4.5.2 SanitizerCoverage (sancov)

When Foundry is compiled with sancov instrumentation (`sancov_edges` or
`sancov_trace_cmp`), native Rust edges (e.g., precompile code paths) are tracked
separately in `RawCallResult::sancov_coverage`. The same AFL binning algorithm
is applied via `merge_sancov_coverage`.

When `sancov_edges` is active, EVM edge coverage is **disabled** to avoid
diluting the signal with Solidity-level edges. When only `sancov_trace_cmp` is
active, EVM edges remain enabled because trace-cmp only contributes dictionary
entries, not edge coverage.

### 4.6 Optimization Mode Updates

For invariants that return `int256`:

1. After every call prefix, the return value is evaluated.
2. If the value is greater than the current best, `optimization_best_value` and
   `optimization_best_sequence` are updated.
3. At the end of the run, if the optimization improved,
   `persist_optimization_state` writes `optimization_best.json` to disk.
4. The best sequence is also added to the corpus so it can be mutated in future
   runs.

---

## 5. Configuration

Corpus behavior is controlled via `FuzzCorpusConfig`, which is nested inside
both `FuzzConfig` and `InvariantConfig`:

| Field                  | Default | Meaning                                                                    |
| ---------------------- | ------- | -------------------------------------------------------------------------- |
| `corpus_dir`           | `None`  | Path to corpus root. If set, coverage-guided fuzzing is enabled.           |
| `corpus_gzip`          | `true`  | Whether to gzip compress entries larger than 4 KiB.                        |
| `corpus_min_mutations` | `5`     | Minimum mutations before an in-memory entry becomes eligible for eviction. |
| `corpus_min_size`      | `0`     | Minimum number of entries to keep in memory regardless of mutation count.  |
| `show_edge_coverage`   | `false` | Display edge coverage metrics in progress output.                          |
| `sancov_edges`         | `false` | Collect sancov edge coverage from native Rust crates.                      |
| `sancov_trace_cmp`     | `false` | Capture comparison operands from sancov for dictionary injection.          |

In `foundry.toml`:

```toml
[fuzz]
corpus_dir = "fuzz_corpus"
corpus_gzip = true
corpus_min_mutations = 5
show_edge_coverage = true

[invariant]
corpus_dir = "invariant_corpus"
corpus_gzip = true
```

---

## 6. Key Data Structures

### `CorpusEntry`

```rust
struct CorpusEntry {
    uuid: Uuid,
    total_mutations: usize,
    new_finds_produced: usize,
    tx_seq: Vec<BasicTxDetails>,
    is_favored: bool,
    timestamp: u64,
}
```

### `WorkerCorpus`

```rust
pub struct WorkerCorpus {
    id: usize,
    in_memory_corpus: Vec<CorpusEntry>,
    history_map: Vec<u8>,
    sancov_history_map: Vec<u8>,
    failed_replays: usize,
    metrics: CorpusMetrics,
    tx_generator: BoxedStrategy<BasicTxDetails>,
    mutation_generator: BoxedStrategy<MutationType>,
    current_mutated: Option<Uuid>,
    config: Arc<FuzzCorpusConfig>,
    new_entry_indices: Vec<usize>,
    last_sync_timestamp: u64,
    worker_dir: Option<PathBuf>,
    last_sync_metrics: CorpusMetrics,
    optimization_best_value: Option<I256>,
    optimization_best_sequence: Vec<BasicTxDetails>,
}
```

### `CorpusMetrics` / `GlobalCorpusMetrics`

```rust
struct CorpusMetrics {
    cumulative_edges_seen: usize,
    cumulative_features_seen: usize,
    corpus_count: usize,
    favored_items: usize,
}
```

`GlobalCorpusMetrics` wraps the same fields in `AtomicUsize` so workers can
safely contribute deltas.

---

## 7. Summary of File Locations in Source

| Component                         | File                                            |
| --------------------------------- | ----------------------------------------------- |
| Corpus manager                    | `crates/evm/evm/src/executors/corpus.rs`        |
| Fuzz executor integration         | `crates/evm/evm/src/executors/fuzz/mod.rs`      |
| Invariant executor integration    | `crates/evm/evm/src/executors/invariant/mod.rs` |
| Coverage merge on `RawCallResult` | `crates/evm/evm/src/executors/mod.rs`           |
| Config definition                 | `crates/config/src/fuzz.rs`                     |
| JSON/gzip I/O helpers             | `crates/common/src/fs.rs`                       |
