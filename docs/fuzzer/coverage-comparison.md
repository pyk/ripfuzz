# Coverage Map Comparison: Medusa, Echidna, Foundry

This document compares how Medusa, Echidna, and Foundry define an "interesting"
corpus item, then proposes a combined approach for raptor's thread-based fuzzer.

## What Makes a Corpus Item "Interesting"?

An "interesting" sequence is one that should be added to the corpus and used for
future mutation. All three fuzzers agree on the core idea: **new execution
behavior = interesting**. But they differ in how precisely they measure
"newness."

---

## Medusa

### Coverage Representation

Medusa uses `CoverageMaps`, a nested map keyed by:

- **Code hash** (`common.Hash`) — identifies contract bytecode
- **Contract address** (`common.Address`) — distinguishes deployments

Each address gets a `ContractCoverageMap` containing `executedMarkers`:

```go
type ContractCoverageMap struct {
    executedMarkers map[uint64]uint64
}
```

A **marker** is a 64-bit integer encoding:

- `jump` / `jumpi`: `bits.RotateLeft64(sourcePC, 32) ^ destPC`
- `revert`: `bits.RotateLeft64(lastPC, 32) ^ REVERT_MARKER_XOR`
- `return`: `bits.RotateLeft64(lastPC, 32) ^ RETURN_MARKER_XOR`
- Contract entrance: `bits.RotateLeft64(ENTER_MARKER_XOR, 32) ^ firstPC`

### What Counts as New

A marker is "new" if its **hit count was previously zero**:

```go
func (cm *ContractCoverageMap) updateCoveredAt(marker uint64) (bool, error) {
    previousVal := cm.executedMarkers[marker]
    newCoverage := previousVal == 0
    cm.executedMarkers[marker] = previousVal + 1
    return newCoverage, nil
}
```

Only the first hit per (contract, address, marker) is "new." Subsequent hits at
the same marker are ignored for corpus decisions.

### Sequence-Level Decision

Medusa checks coverage **after every call** in a sequence:

```go
// In testNextCallSequence()
err = fw.fuzzer.corpus.CheckSequenceCoverageAndUpdate(
    currentlyExecutedSequence,
    fw.getNewCorpusCallSequenceWeight(),
    true,
)
```

If the **last call** in the sequence produced new coverage, the **entire
sequence** is saved to the corpus. The check is done per-call, so a sequence can
be saved even if only its final call discovered something new.

### Summary

| Aspect         | Medusa                                              |
| -------------- | --------------------------------------------------- |
| Granularity    | Per-contract, per-deployment, per-control-flow-edge |
| Data structure | `map[codeHash]map[address]map[marker]hitCount`      |
| "New" means    | First hit for a given marker                        |
| Hit counts     | Tracked but ignored for "interesting" decision      |
| Sequence saved | Entire sequence if last call had new coverage       |

---

## Echidna

### Coverage Representation

Echidna uses a `CoverageMap` keyed by **compile-time code hash** (`W256`):

```haskell
type CoverageMap = Map W256 (IOVector CoverageInfo)
type CoverageInfo = (OpIx, StackDepths, TxResults)
```

For every program counter (PC), Echidna stores:

- **`OpIx`** — source-level operation index (from Solidity source map)
- **`StackDepths`** — packed bitset of call stack depths at which this PC was
  hit
- **`TxResults`** — packed bitset of transaction result types (Stop, Revert,
  etc.)

### What Counts as New

Echidna tracks three dimensions at each PC:

1. **Operation index** — has this source-level operation been hit?
2. **Call depth** — has this PC been hit at this specific call-frame depth?
3. **Transaction result** — has this PC been hit with this specific outcome?

New coverage = a new combination of (PC, depth, result) that has not been seen:

```haskell
(_, depths, results) | depth < 64 && not (depths `testBit` depth) -> do
    VMut.write vec pc (opIx, depths `setBit` depth, results `setBit` fromEnum Stop)
    writeIORef covContextRef (True, Just (vec, pc))
```

If a PC was hit before but at a different depth or with a different result, that
is **also new coverage**.

### Sequence-Level Decision

Echidna runs `execTxOptC` for **every transaction** in the sequence. After each
transaction, if `grew` (new coverage found), the worker sets
`newCoverage = True`. After the full sequence, if `newCoverage` is true, the
entire sequence is added to the corpus:

```haskell
newCoverage <- gets (.newCoverage)
when newCoverage $ do
    -- add sequence to corpus
```

### Summary

| Aspect         | Echidna                                               |
| -------------- | ----------------------------------------------------- |
| Granularity    | Per-PC, per-call-depth, per-transaction-result        |
| Data structure | `Map codehash (Vector (opIx, depthBits, resultBits))` |
| "New" means    | New PC-depth-result combination                       |
| Hit counts     | Not tracked; boolean per dimension                    |
| Sequence saved | Entire sequence if any call had new coverage          |

---

## Foundry

### Coverage Representation

Foundry uses a **flat byte array** (65,536 bytes) for edge coverage, similar to
AFL's classic bitmap:

```rust
const COVERAGE_MAP_SIZE: usize = 65536;
let history_map: Vec<u8> = vec![0u8; COVERAGE_MAP_SIZE];
```

Each byte represents a PC index. The value is not a raw hitcount but an **AFL
hitcount bucket**:

```rust
let bucket = match *curr {
    0 => 0,
    1 => 1,
    2 => 2,
    3 => 4,
    4..=7 => 8,
    8..=15 => 16,
    16..=31 => 32,
    32..=127 => 64,
    128..=255 => 128,
};
```

### What Counts as New

Foundry distinguishes between two kinds of "new":

1. **New edge** — `history[pc]` was `0`, now has a non-zero bucket
2. **New feature** — `history[pc]` had bucket `X`, now has bucket `Y > X`

```rust
if *hist < bucket {
    if *hist == 0 {
        is_edge = true;   // new edge
    }
    *hist = bucket;
    new_coverage = true;  // new edge OR new feature
}
```

A **feature** is a higher hitcount bucket for a previously seen edge. This
captures the intuition that "hitting the same branch 8 times instead of 2 times"
is different execution behavior and may be worth preserving.

### Sequence-Level Decision

Foundry checks coverage **after every call** in an invariant run. If a call
produces new coverage, the flag `current_run.new_coverage = true` is set. After
the run completes, if new coverage was found, the **entire run's call sequence**
is saved:

```rust
corpus_manager.process_inputs(
    &current_run.inputs,
    current_run.new_coverage,
    optimization,
);
```

Foundry also tracks **favorability** per corpus entry:

```rust
let is_favored = (corpus.new_finds_produced as f64 / corpus.total_mutations as f64)
    > FAVORABILITY_THRESHOLD; // 0.3
```

Favored entries are preferred for mutation. This is an adaptive weighting
mechanism that rewards entries that consistently produce new finds.

### Summary

| Aspect         | Foundry                                             |
| -------------- | --------------------------------------------------- |
| Granularity    | Per-PC with AFL hitcount buckets                    |
| Data structure | `Vec<u8; 65536>`                                    |
| "New" means    | New edge (first hit) OR new feature (higher bucket) |
| Hit counts     | Bucketed into 8 levels (AFL-style)                  |
| Sequence saved | Entire run if any call had new coverage             |
| Extra          | Favorability tracking per entry                     |

---

## Comparison Matrix

| Dimension                      | Medusa                            | Echidna                     | Foundry                      |
| ------------------------------ | --------------------------------- | --------------------------- | ---------------------------- |
| Key unit                       | Control-flow edge (src-dst pair)  | PC-depth-result triple      | PC                           |
| Per-contract tracking          | Yes (nested map)                  | Yes (codehash key)          | No (flat array)              |
| Per-deployment tracking        | Yes (address key)                 | No                          | No                           |
| Call depth sensitivity         | Implicit via src-dst              | Explicit (64 bits)          | No                           |
| Transaction result sensitivity | Yes (revert/return/enter markers) | Yes (tx result bits)        | No                           |
| Hitcount sensitivity           | No (boolean first-hit)            | No                          | Yes (AFL buckets)            |
| Coverage map size              | Unbounded (sparse maps)           | Unbounded (sparse maps)     | Fixed 65,536                 |
| Sequence saved                 | Entire sequence                   | Entire sequence             | Entire sequence              |
| Adaptive weighting             | Weighted random (static weight)   | Weighted by sequence number | Favorability (30% threshold) |

---

## Combined Approach for Raptor

### What to Adopt from Each

**From Medusa:**

- Track **control-flow edges** (jumps) not just PCs. A jump from PC `A` to PC
  `B` is different from a jump from PC `A` to PC `C`. This captures branch
  direction.
- Track **reverts and returns** as distinct coverage events. A sequence that
  causes a revert at a new location is interesting.

**From Echidna:**

- Track **call depth** dimension. Reaching the same code at a deeper call depth
  (e.g., through a callback or reentrancy) is different behavior.
- Track **transaction result** dimension. A PC hit in a successful call vs. a
  reverted call is different behavior.

**From Foundry:**

- Use **AFL-style hitcount bucketing**. A sequence that hits the same edge 16
  times instead of 2 times may exercise different state-dependent paths.
- Use **favorability tracking**. Reward corpus items that consistently produce
  new coverage when mutated. This guides mutation selection toward productive
  parents.
- Use **right-sized per-contract maps** sized to each bytecode's length,
  rather than a single fixed 65,536-byte buffer. This eliminates PC collision
  across contracts and enables source-level reporting.

### Proposed Coverage Map for Raptor

Coverage is stored **per-contract**, keyed by bytecode hash, so that every
hit can be mapped back to source code. Each contract gets a right-sized map:

```rust
/// Global coverage state, stored in the Corpus.
pub struct CoverageMap {
    /// Per-contract coverage data.
    contracts: HashMap<ContractId, ContractCoverage>,
}

/// Coverage data for a single contract.
pub struct ContractCoverage {
    /// Per-PC hitcount buckets. Length equals bytecode length.
    /// Each byte is an AFL bucket (0, 1, 2, 4, 8, 16, 32, 64, 128).
    edges: Vec<u8>,

    /// Per-PC call-depth bitset. Each u64 is a bitset of depths at which this PC was hit.
    /// Only tracked for the first 1,024 PCs to limit memory.
    depths: Vec<u64>,

    /// Per-PC revert bitset. One bit per PC (packed into u64 words).
    /// A bit is set only for the **final PC** of a call that reverted,
    /// matching Medusa's `REVERT_MARKER_XOR` approach.
    reverts: Vec<u64>,
}
```

This is a **hybrid**:

- The **edges** array gives us Foundry-style AFL bucket coverage for all PCs,
  sized exactly to the contract's bytecode length.
- The **depths** array gives us Echidna-style call-depth sensitivity for the
  most frequently executed PCs (first 1,024). This captures reentrancy depth
  without exploding memory.
- The **reverts** array gives us Medusa-style revert tracking. Only the final
  PC of a reverted call is marked, not every PC executed during that call.

### "Interesting" Decision

A sequence is interesting if any of these are true:

1. **New edge** — a PC was hit for the first time (bucket went from 0 to >0).
2. **New feature** — a PC's bucket increased to a higher AFL level.
3. **New depth** — a PC (in the first 1,024) was hit at a call depth never
   before seen for that PC.
4. **New revert** — a PC was hit in a reverted call for the first time.

```rust
pub struct CoverageUpdate {
    pub new_edges: usize,
    pub new_features: usize,
    pub new_depths: usize,
    pub new_reverts: usize,
}

impl CoverageMap {
    pub fn merge(&mut self, local: &LocalCoverage) -> CoverageUpdate {
        // ... merge edges, depths, reverts ...
    }

    pub fn is_interesting(&self, update: &CoverageUpdate) -> bool {
        update.new_edges > 0
            || update.new_features > 0
            || update.new_depths > 0
            || update.new_reverts > 0
    }
}
```

### Sequence-Level Decision

Per-sequence coverage is collected locally during execution. After each call,
the worker checks if the **last call** produced new coverage. If so, it
immediately flags the sequence as interesting.

After the full sequence, the worker sends the **entire sequence** to the Corpus
if it was flagged as interesting. This matches all three fuzzers.

### Favorability (Adaptive Weighting)

Inspired by Foundry, each `CorpusItem` tracks:

```rust
pub struct CorpusItem {
    pub calls: Vec<Call>,
    pub weight: u64,
    pub total_mutations: u64,
    pub new_finds_produced: u64,
}
```

When selecting an item for mutation, the chooser uses weight proportional to:

```
weight = base_weight + (new_finds_produced * 10)
```

Items that consistently produce new coverage get higher weight. Items that
produce no new finds after many mutations get deprioritized.

### Per-Worker Local Map

Each worker owns a local coverage map that mirrors the global shape:

```rust
pub struct LocalCoverage {
    contracts: HashMap<ContractId, LocalContractCoverage>,
}

pub struct LocalContractCoverage {
    edges: Vec<u8>,       // sized to bytecode length
    depths: Vec<u64>,     // 1,024 entries (max)
    reverts: Vec<u64>,    // sized to bytecode length / 64
}
```

After each call, the `CoverageInspector` writes into the local map. After the
sequence, the worker calls `corpus.merge(&local)` which returns a
`CoverageUpdate`. If interesting, the sequence is added.

### Why This Is Better Than Any Single Approach

| Feature          | Medusa | Echidna | Foundry | Raptor (combined) |
| ---------------- | ------ | ------- | ------- | ----------------- |
| Branch direction | Yes    | No      | No      | Yes (via edges)   |
| Call depth       | No     | Yes     | No      | Yes (limited)     |
| Revert tracking  | Yes    | Yes     | No      | Yes               |
| Hitcount buckets | No     | No      | Yes     | Yes               |
| Favorability     | No     | No      | Yes     | Yes               |
| Fixed memory     | No     | No      | Yes     | Yes               |
| Fast merge       | No     | No      | Yes     | Yes               |

Raptor's combined approach captures **more dimensions of execution behavior**
without the unbounded memory growth of Medusa/Echidna's sparse maps. The
right-sized per-contract maps mean:

- No dynamic allocation during merge.
- O(n) merge where n = bytecode length (fast).
- No `unsafe` code (no shared memory needed).
- Predictable memory per worker (~12 KB for a 4,000-byte contract).

### Open Question: Should We Also Track Return vs. Success?

Medusa tracks `RETURN_MARKER_XOR` separately from `REVERT_MARKER_XOR`. Echidna
tracks `TxResults` (Stop, Revert, etc.). In practice, for a single-contract fuzz
target, the distinction between "returned normally" and "reverted" is the most
important. We already track reverts. Adding "returned" tracking would be
redundant because most non-reverting calls return normally.

**Decision:** Track reverts only. Do not track returns separately.
