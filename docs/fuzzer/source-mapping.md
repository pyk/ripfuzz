# Source Mapping for Coverage Data

This document describes how coverage data collected during fuzzing should map
back to Solidity source code, comparing approaches from Medusa, Echidna, and
Foundry.

## The Problem

A fuzzer collects coverage at the **bytecode level** (program counters, jump
destinations, hit counts). Users need to see coverage at the **source level**
(statements, branches, lines, functions). The bridge between these two worlds is
the **Solidity source map** produced by the compiler.

Without a clear mapping, coverage data is just a flat array of numbers. With a
proper mapping, every hit in the coverage map can be traced back to:

- Which Solidity source file
- Which line and column range
- Which AST node (statement, branch, function)

## What Raptor Already Has

Raptor reads Foundry artifact JSON files. These artifacts already contain source
maps:

```json
{
  "abi": [...],
  "bytecode": {
    "object": "60806040...",
    "sourceMap": "110:117:0:-:0;;;;;;;;;;;;;;;;;;;..."
  },
  "deployedBytecode": {
    "object": "60806040...",
    "sourceMap": "137:88:-:0;;;:::i;:::-;;;201:3;..."
  }
}
```

The `sourceMap` field is present but currently **ignored** by raptor.

## Solidity Source Map Format

The source map is a semicolon-separated list where each entry corresponds to one
instruction in the bytecode. Each entry has the form:

```
s:l:f:j:m
```

| Field | Meaning                                                     |
| ----- | ----------------------------------------------------------- |
| `s`   | byte offset in source file (omitted = same as previous)     |
| `l`   | length in source bytes (omitted = same as previous)         |
| `f`   | source file index (omitted = same as previous)              |
| `j`   | jump type: `i` (into function), `o` (return), `-` (regular) |
| `m`   | modifier depth (omitted = same as previous)                 |

Example: `137:88:0:-:0` means "instruction corresponds to source bytes 137-225
(88 bytes long) in source file 0, regular jump, modifier depth 0."

## How Foundry Does It

Foundry has the most complete source-to-bytecode coverage pipeline:

### 1. Data Model

```rust
pub struct CoverageReport {
    /// Source file paths keyed by (compiler_version, source_id).
    source_paths: HashMap<(Version, usize), PathBuf>,

    /// AST-level coverage items (statements, branches, functions, lines).
    analyses: HashMap<Version, SourceAnalysis>,

    /// Anchors map source items to bytecode instructions.
    anchors: HashMap<ContractId, (Vec<ItemAnchor>, Vec<ItemAnchor>)>,

    /// Bytecode hit counts from execution.
    bytecode_hits: HashMap<ContractId, HitMap>,

    /// Source maps for each contract.
    source_maps: HashMap<ContractId, (SourceMap, SourceMap)>,
}
```

### 2. Two-Direction Mapping

Foundry maps in **both directions**:

**Source -> Bytecode (Anchors):**

1. Parse Solidity AST to find coverage items (statements, branches, etc.)
2. For each item, find the first bytecode instruction whose source map entry
   falls within the item's source range
3. Store as `ItemAnchor { instruction: PC, item_id }`

**Bytecode -> Source (Source Map):**

1. Parse the raw source map string from the artifact
2. For any bytecode PC, look up the corresponding source file, offset, and
   length
3. Map back to the original source file and line numbers

### 3. Hit Reporting

```rust
pub fn add_hit_map(
    &mut self,
    contract_id: &ContractId,
    hit_map: &HitMap,
    is_deployed_code: bool,
) {
    // Bytecode-level: merge hit counts
    self.bytecode_hits
        .entry(contract_id.clone())
        .and_modify(|m| m.merge(hit_map))
        .or_insert_with(|| hit_map.clone());

    // Source-level: use anchors to propagate hits to coverage items
    let anchors = if is_deployed_code { &anchors.1 } else { &anchors.0 };
    for anchor in anchors {
        if let Some(hits) = hit_map.get(anchor.instruction) {
            self.analyses
                .get_mut(&contract_id.version)
                .and_then(|items| items.get_mut(anchor.item_id))
                .expect("anchor exists")
                .hits += hits.get();
        }
    }
}
```

## How Echidna Does It

Echidna's coverage is coarser than Foundry's but still source-aware:

```haskell
type CoverageInfo = (OpIx, StackDepths, TxResults)
```

### Key Design

- **`OpIx`** = source-level operation index, obtained via `vmOpIx` which uses
  the source map to map the current PC back to an index in the source-level AST.
- The coverage vector is indexed by **compile-time code hash**, so different
  deployments of the same contract share the same coverage map.
- Echidna's `SourceMapping` module handles immutables: runtime bytecode may
  differ from compile-time bytecode, so a `CodehashMap` resolves the runtime
  codehash back to the compile-time codehash.

### Mapping Chain

```
Runtime codehash -> CodehashMap -> Compile-time codehash -> CoverageMap -> OpIx
```

For source reporting, Echidna merges init and runtime coverage maps and uses
`DappInfo` to resolve source locations from operation indices.

## How Medusa Does It

Medusa focuses on **branch-level** coverage with source mapping done at report
time, not during fuzzing:

### Coverage Collection (Fuzzing Time)

```go
type CoverageMaps struct {
    maps map[common.Hash]map[common.Address]*ContractCoverageMap
}

type ContractCoverageMap struct {
    executedMarkers map[uint64]uint64
}
```

Markers encode control-flow edges:

- Jump: `RotateLeft64(srcPC, 32) ^ dstPC`
- Revert: `RotateLeft64(lastPC, 32) ^ REVERT_MARKER_XOR`
- Return: `RotateLeft64(lastPC, 32) ^ RETURN_MARKER_XOR`

### Source Mapping (Report Time)

Medusa uses bytecode metadata hashing to identify contracts. During report
generation, it:

1. Resolves bytecode hashes to contract definitions
2. Uses compilation artifacts to map PCs back to source
3. Produces source-level coverage reports

Medusa does **not** track source-level operation indices during fuzzing; the
marker approach is entirely bytecode-centric.

## The Map We Need

For raptor, the mapping chain should be explicit and reversible:

```
┌──────────────────────────────────────────────────────────────┐
│                         SOURCE LEVEL                          │
│  Source file -> AST node (statement, branch, function, line)   │
└──────────────────────┬───────────────────────────────────────┘
                       │ anchors (source -> bytecode)
                       │ source_map (bytecode -> source)
┌──────────────────────▼───────────────────────────────────────┐
│                       BYTECODE LEVEL                          │
│  Contract -> (init_bytecode, runtime_bytecode) -> PC -> hit   │
└──────────────────────────────────────────────────────────────┘
```

### Proposed Data Model for Raptor

```rust
/// Parsed source map for a contract's bytecode.
pub struct SourceMap {
    /// Raw entries, one per instruction.
    pub entries: Vec<SourceMapEntry>,
    /// The source file path.
    pub source_path: PathBuf,
    /// Contract name.
    pub contract_name: String,
}

pub struct SourceMapEntry {
    pub source_offset: usize,
    pub source_length: usize,
    pub source_file_index: usize,
    pub jump_type: JumpType,
    pub modifier_depth: usize,
}

pub enum JumpType {
    Regular,
    Into,
    Out,
}
```

### Contract Artifact Extension

Extend `ContractArtifact` to carry source maps:

```rust
pub struct ContractArtifact {
    pub contract_name: String,
    pub initcode: Bytes,
    pub runtime: Bytecode,
    pub abi: JsonAbi,
    pub properties: Vec<([u8; 4], String)>,
    pub initcode_map: HashMap<Bytes, (String, JsonAbi)>,

    // NEW: source maps for both init and runtime bytecode
    pub init_source_map: Option<SourceMap>,
    pub runtime_source_map: Option<SourceMap>,
}
```

The source maps are **parsed at build time** from the Foundry artifact JSON.
They are immutable during fuzzing and shared across all workers as `Arc`.

### Coverage Map Per Contract

Instead of a flat 65,536-byte array, coverage should be **per-contract** so that
PCs can be resolved back to source:

```rust
/// Coverage data for a single contract.
pub struct ContractCoverage {
    /// Contract identifier (bytecode hash or name).
    pub contract_id: ContractId,
    /// Bytecode-level hit map: PC -> AFL bucket.
    pub edges: Vec<u8>,
    /// Call-depth sensitivity for hot PCs (first 1,024).
    pub depths: Vec<u64>,
    /// Revert tracking.
    pub reverts: Vec<u64>,
}

/// Global coverage across all contracts.
pub struct CoverageMap {
    pub contracts: HashMap<ContractId, ContractCoverage>,
}
```

### Inspector Changes

The `CoverageInspector` needs to know **which contract** is executing:

```rust
pub struct CoverageInspector<'a> {
    map: &'a mut CoverageMap,
    current_contract: Option<ContractId>,
}

impl<'a, CTX> Inspector<CTX, EthInterpreter> for CoverageInspector<'a> {
    fn step(&mut self, interp: &mut Interpreter<EthInterpreter>, context: &mut CTX) {
        let pc = interp.bytecode.pc() as usize;
        let contract_id = self.current_contract.as_ref().expect("contract set");

        if let Some(coverage) = self.map.contracts.get_mut(contract_id) {
            let idx = pc % coverage.edges.len();
            coverage.edges[idx] = coverage.edges[idx].saturating_add(1);
        }
    }
}
```

The contract ID is set in `initialize_interp` by reading the current
interpreter's bytecode hash.

### Source Resolution (Post-Fuzzing)

After a campaign completes, raptor can optionally produce a coverage report:

```rust
pub fn resolve_coverage_to_source(
    coverage: &CoverageMap,
    artifact: &ContractArtifact,
) -> SourceCoverageReport {
    let mut report = SourceCoverageReport::default();

    for (contract_id, contract_cov) in &coverage.contracts {
        let source_map = artifact.runtime_source_map.as_ref()
            .expect("source map required for reporting");

        for (pc, bucket) in contract_cov.edges.iter().enumerate() {
            if *bucket == 0 { continue; }

            // Map PC back to source location via source map
            if let Some(entry) = source_map.entries.get(pc) {
                report.add_hit(
                    &source_map.source_path,
                    entry.source_offset,
                    entry.source_length,
                    *bucket,
                );
            }
        }
    }

    report
}
```

### Why Per-Contract Coverage Instead of Flat

| Flat Array (current)          | Per-Contract Coverage (proposed)                       |
| ----------------------------- | ------------------------------------------------------ |
| One 65K map for all           | One map per contract, sized to bytecode                |
| PC collision across contracts | No collision; each contract has its own PC space       |
| Cannot map to source          | Direct mapping: PC -> source_map[PC] -> source         |
| Simple, fast merge            | Slightly more complex, but still fast                  |
| No contract identity          | Contract identity preserved for multi-contract targets |

For raptor's typical use case (single target contract), the per-contract map is
essentially a right-sized array instead of a fixed 65K buffer. The source map
gives us line-level coverage for free.

### Memory Cost

For a contract with 4,000 bytes of runtime bytecode:

- Edges: 4,000 bytes
- Depths: 1,024 \* 8 = 8,192 bytes (if depth tracking enabled)
- Reverts: 4,000 / 8 = 500 bytes (bit-packed)
- Source map entries: 4,000 _ (5 _ 4) = ~80,000 bytes (if stored densely)

Total: ~92 KB per contract. With `Arc<SourceMap>`, all workers share the source
map. Each worker only needs its own local `ContractCoverage` (~12 KB).

### Open Decision: Should Source Maps Be Optional?

If a user does not care about source-level coverage reports, the source map
parsing could be skipped. However:

- Foundry always produces source maps in artifacts when `build_info = true`
- Parsing is a one-time cost at build time
- The extra memory is small for typical contracts

**Decision:** Always parse and store source maps. They are essential for:

1. Coverage reports
2. Debugging which branches were explored
3. Future features (e.g., "show uncovered lines")

## Summary of Mapping Chain

```
Fuzzing time:
  EVM execution -> PC -> ContractCoverage.edges[PC] -> bucket

Report time:
  ContractCoverage.edges[PC] > 0
    -> SourceMap.entries[PC] -> (source_file, offset, length)
    -> Source file text -> line and column range
    -> Optional: AST anchors -> statement/branch/function identity
```

This is the same approach Foundry uses, simplified for raptor's single-contract
focus. Echidna and Medusa both have similar pipelines but use different internal
representations (operation indices vs. branch markers).
