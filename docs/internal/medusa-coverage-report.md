# Medusa Coverage Report & Optimizer Bytecode

This document summarizes how [medusa](https://github.com/crytic/medusa) handles
coverage reporting when the Solidity optimizer is enabled, based on the source
code in `external/crytic/medusa`.

---

## 1. Does Medusa Support Coverage Reports with the Optimizer Enabled?

**Yes.** Medusa does not disable or prohibit coverage reporting when optimized
bytecode is used. The fuzzer generates coverage reports from whatever
compilation artifacts (bytecode + source maps) the compiler provides. There is
no explicit optimizer check or special branch in the coverage pipeline that
rejects optimized builds.

However, coverage accuracy depends entirely on the quality of the
compiler-generated source maps (`srcmap` / `srcmap-runtime`). When the optimizer
is enabled, the compiler may inline, reorder, or eliminate code, which can make
source maps less precise. Medusa consumes the source maps as-is and does not
have additional heuristics to "undo" optimization.

---

## 2. What Are the Default Optimizer Settings for Medusa?

**Medusa does not specify default optimizer settings.** It does not invoke
`solc` with `--optimize` or `--optimize-runs` flags, nor does it configure
Foundry/Hardhat compiler settings internally.

- **Solc platform** (`platforms/solc.go`): Medusa calls
  `solc <target> --combined-json <outputs>` with output options such as
  `abi,ast,bin,bin-runtime,srcmap,srcmap-runtime`. No optimizer flags are added.
- **Crytic-compile platform** (`platforms/crytic_compile.go`): Medusa delegates
  compilation to `crytic-compile`, which in turn uses the project's own build
  configuration (e.g., `foundry.toml`, `hardhat.config.js`). The optimizer
  settings are whatever the user configured in their build system.

In short: optimizer settings are external to Medusa and come from the user's
compiler / build tool configuration.

---

## 3. How Medusa Generates Coverage Reports with Optimizer-Enabled Bytecode

### 3.1 Bytecode-Level Coverage Tracking

Medusa's `CoverageTracer` (`fuzzing/coverage/coverage_tracer.go`) does not track
source lines directly during execution. Instead, it tracks **bytecode-level
execution markers**:

- **Contract entrance**: `ENTER_MARKER_XOR ^ firstPC`
- **Jumps**: `sourcePC ^ destinationPC` (for `JUMP` and `JUMPI`)
- **Returns**: `sourcePC ^ RETURN_MARKER_XOR`
- **Reverts**: `sourcePC ^ REVERT_MARKER_XOR`

These markers are stored in `CoverageMaps` keyed by contract address and a
**code hash** (`getContractCoverageMapHash`). Because coverage is recorded at
the EVM trace level, it works regardless of whether the bytecode is optimized or
not.

### 3.2 Source Map Parsing

When Medusa generates a coverage report, it parses the `srcmap` and
`srcmap-runtime` strings from the compilation artifacts
(`compilation/types/source_maps.go`). Each source map element maps an
**instruction index** (not program counter) to:

- `Offset`: byte offset in the source file
- `Length`: byte length of the source range
- `SourceUnitID`: source file identifier
- `JumpType`: `i` (in), `o` (out), `-` (within), or empty

### 3.3 Mapping Bytecode to Source Lines

The `analyzeContractSourceCoverage` function
(`fuzzing/coverage/source_analysis.go`) performs the mapping:

1. Parse source maps for init and runtime bytecode.
2. Filter out overlapping (superset) source map entries. This prevents an entire
   function definition from being marked as "active" when only a small inner
   range is actually mapped.
3. Skip entries with `SourceUnitID == -1`. These represent **compiler-generated
   inline code** (e.g., Yul snippets inserted by the optimizer) that do not map
   to any user source file.
4. Skip entries whose `SourceUnitID` is not present in the compilation's
   `SourceIdToPath` map. This can happen for **generated sources** (e.g., Yul
   optimizer output files) that Medusa does not currently fetch.
5. For each valid source map element, look up the corresponding source line
   using binary search on `CumulativeOffsetByLine`.
6. Mark the line as `IsActive` and record hit counts (`SuccessHitCount`,
   `RevertHitCount`).

### 3.4 Hit Count Computation

`determineLinesCovered` (`source_analysis.go`) converts the raw execution
markers into per-instruction-index hit counts:

- It builds an `indexToOffset` lookup from the bytecode (handling
  variable-length `PUSH` operands).
- It reconstructs entry/exit flow from markers: `enterCount`, `revertCount`,
  `allLeaveCount`.
- It calculates successful hits as `hit + enterCount - revertCount` and reverted
  hits as `revertCount`.

Because this operates on the deployed bytecode, it is agnostic to optimizer
settings.

---

## 4. How Medusa Provides (Reasonably) Accurate Coverage Reports with Optimized Bytecode

### 4.1 Library Linking Before Analysis

If a contract uses libraries, its bytecode contains placeholders (`__$...$__`).
Medusa deploys libraries first, then calls `LinkBytecodes` on the compilation
artifacts **before** generating the final coverage report (`fuzzing/fuzzer.go`).
This ensures that:

- The bytecode used for coverage lookup matches the bytecode that actually ran
  on-chain.
- The `indexToOffset` and source map remain aligned with the linked bytecode.

### 4.2 Contract Identification via Metadata Hash

`getContractCoverageMapHash` (`fuzzing/coverage/coverage_maps.go`) tries to
identify runtime contracts by extracting the **CBOR metadata hash** (IPFS /
Swarm) appended by the Solidity compiler. This is more reliable than hashing the
full bytecode because:

- Constructor arguments are not part of the runtime hash.
- Immutables do not affect the metadata hash.

However, some builds (e.g., monorepos with identical compiler settings) can
produce different contracts with **identical metadata**. When this happens,
Medusa logs a warning and suggests setting `USE_FULL_BYTECODE=1`. This
environment variable forces Medusa to hash the full stripped bytecode instead of
the metadata hash, avoiding collisions at the cost of potentially breaking on
contracts with immutables.

### 4.3 Integrity Checks and Warnings

`determineLinesCovered` contains several invariant checks that warn the user if
the coverage report is likely inaccurate:

- Overflow / underflow during hit-count arithmetic.
- `hit + enterCount != allLeaveCount` at a `JUMP` / `RETURN` / `STOP`.
- Nonzero final hit count after traversing all instructions.

When any of these fire, Medusa prints:

```
WARNING: ... The coverage report will be inaccurate.
Try setting USE_FULL_BYTECODE=1 in your environment and rerunning medusa.
```

These checks are not optimizer-specific, but they can be triggered more often
when bytecode is heavily transformed (e.g., optimized, inlined, or when source
maps are imprecise).

### 4.4 Handling of Generated / Inline Code

Medusa explicitly skips source map entries that have no user-visible source
file:

- `SourceUnitID == -1` → compiler-generated inline code.
- `SourceUnitID` not in `SourceIdToPath` → generated sources (e.g., Yul
  optimizer output files).

This means optimizer-generated snippets are **not counted against source
coverage**. Medusa only reports coverage for lines that the compiler explicitly
mapped back to the original Solidity source.

### 4.5 Accuracy Limits

Medusa does **not** attempt to reverse-engineer optimized bytecode or
reconstruct original control flow. Its accuracy is bounded by the compiler's
source map fidelity:

- If the optimizer inlines a function and the source map points the inlined
  instructions back to the original function definition, Medusa will report the
  original line as covered.
- If the optimizer eliminates code and the corresponding source map entries are
  missing, Medusa will not report those lines as active (they will appear
  uncovered / non-executable).
- If the optimizer produces generated Yul files that are not included in the
  `sources` section of the compiler output, Medusa silently skips them.

In practice, Medusa provides **as-accurate-as-possible** coverage given the
compiler's source map output, but it does not guarantee perfect line-level
accuracy for highly optimized code.

---

## Key Files

| File                                     | Role                                                                                      |
| ---------------------------------------- | ----------------------------------------------------------------------------------------- |
| `fuzzing/coverage/coverage_tracer.go`    | Records bytecode-level execution markers (jumps, reverts, returns, entrances)             |
| `fuzzing/coverage/coverage_maps.go`      | Stores and merges coverage maps; hashes contracts for lookup                              |
| `fuzzing/coverage/source_analysis.go`    | Parses source maps, maps instruction indexes to source lines, filters overlapping entries |
| `fuzzing/coverage/report_generation.go`  | Generates HTML and LCOV reports from `SourceAnalysis`                                     |
| `compilation/types/source_maps.go`       | Parses `srcmap` / `srcmap-runtime` strings into structured elements                       |
| `compilation/types/compiled_contract.go` | Holds bytecode, ABI, and source maps for a contract                                       |
| `fuzzing/fuzzer.go`                      | Orchestrates deployment, library linking, and final report generation                     |
| `platforms/solc.go`                      | Solc compilation platform (no optimizer flags)                                            |
| `platforms/crytic_compile.go`            | Crytic-compile platform (delegates to user's build config)                                |
