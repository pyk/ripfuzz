# TargetContractBasic Coverage: Optimizer-Enabled vs Optimizer-Disabled

This document investigates why coverage reports differ between
optimizer-disabled and optimizer-enabled builds of `TargetContractBasic.sol`,
using the fixture projects at:

- `fixtures/coverage-report-optimizer-disabled`
- `fixtures/coverage-report-optimizer-enabled`

## 1. Context

Two fixture projects compile the same Solidity source but differ in their
`foundry.toml` optimizer settings:

| Setting          | Disabled fixture | Enabled fixture |
| ---------------- | ---------------- | --------------- |
| `optimizer`      | `false`          | `true`          |
| `optimizer_runs` | —                | `1000`          |
| `solc_version`   | `0.8.34`         | `0.8.34`        |

Each fixture has a `reports/` directory containing expected `lcov` output. The
expected reports are identical across both fixtures — the goal is for coverage
to match regardless of optimizer.

## 2. The Test

`optimizer_enabled_target_contract_basic_call_once` (in
`src/evm/coverage/reporter.rs`) deploys `TargetContractBasic` from the
optimizer-enabled fixture, calls `addAndSub(123, 123)` once, and compares the
generated `lcov` report against
`fixtures/coverage-report-optimizer-enabled/reports/TargetContractBasicOnce.info`.

The test **fails** because the generated report differs from the expected
report.

## 3. Source Contract

```solidity
contract TargetContractBasic is RaptorFuzz {
    uint256 public latestValue;

    constructor() {
        latestValue = 0;                              // line 10
    }

    function addAndSub(uint256 a, uint256 b) external returns (uint256) {
        uint256 result = add(a, b);                   // line 14
        result = sub(result, b);                      // line 15
        return result;                                // line 16
    }

    function add(uint256 a, uint256 b) internal returns (uint256) {
        latestValue = a + b;                          // line 20
        return latestValue;                           // line 21
    }

    function sub(uint256 a, uint256 b) internal returns (uint256) {
        latestValue = a - b;                          // line 25
        return latestValue;                           // line 26
    }

    function div(uint256 a, uint256 b) internal returns (uint256) {
        latestValue = a / b;                          // line 31
        return latestValue;                           // line 32
    }
}
```

## 4. The Failure: Side-by-Side Comparison

| Line  | Source                     | Expected (disabled) | Actual (enabled) | Δ   |
| ----- | -------------------------- | ------------------- | ---------------- | --- |
| 9     | `constructor()`            | 1                   | 1                | —   |
| 10    | `latestValue = 0`          | 1                   | 1                | —   |
| 13    | `addAndSub(...)` signature | 1                   | **3**            | +2  |
| 14    | `result = add(a, b)`       | 1                   | 1                | —   |
| 15    | `result = sub(result, b)`  | 1                   | 1                | —   |
| 16    | `return result`            | 1                   | **0**            | −1  |
| 19    | `add(...)` signature       | 1                   | **2**            | +1  |
| 20    | `latestValue = a + b`      | 1                   | **2**            | +1  |
| 21    | `return latestValue`       | 1                   | **0**            | −1  |
| 24    | `sub(...)` signature       | 1                   | 1                | —   |
| 25    | `latestValue = a - b`      | 1                   | 1                | —   |
| 26    | `return latestValue`       | 1                   | **0**            | −1  |
| 30-32 | `div(...)` (not called)    | 0                   | 0                | —   |

Three patterns stand out:

1. **Return statement lines (16, 21, 26) show 0 hits** — they were hit in the
   unoptimized build but register nothing in the optimized build.
2. **Some lines show inflated hit counts** — line 13 shows 3 instead of 1, lines
   19-20 show 2 instead of 1.
3. **Function hit counts (`FNDA`) are also affected** — `add` reports `FNDA:2`
   in the optimized build (should be 1).

## 5. Root Cause

### 5.1 Bytecode and Source Map Statistics

| Metric                    | Disabled | Enabled |
| ------------------------- | -------- | ------- |
| Deployed bytecode size    | 590 B    | 338 B   |
| Source map entries        | 228      | 93      |
| Jump-in (`j='i'`) entries | 9        | 6       |

The optimizer shrinks bytecode by 43% and source map entries by 59%.

### 5.2 Missing Source Map Entries for Return Statements

In the **optimizer-disabled** build, each `return` statement has its own
dedicated source map entries:

| Function    | Line | Source map entries                                  |
| ----------- | ---- | --------------------------------------------------- |
| `addAndSub` | 16   | `s=392 l=6` and `s=385 l=13` (both map to line 16)  |
| `add`       | 21   | `s=519 l=11` and `s=512 l=18` (both map to line 21) |
| `sub`       | 26   | `s=651 l=11` and `s=644 l=18` (both map to line 26) |

In the **optimizer-enabled** build, **all of these entries are absent**. The
return statements are folded into broad function-range entries:

| Function    | Broad range entry | Lines covered           |
| ----------- | ----------------- | ----------------------- |
| `addAndSub` | `s=238 l=167`     | 13–17 (entire function) |
| `add`       | `s=411 l=126`     | 19–22 (entire function) |
| `sub`       | `s=543 l=126`     | 24–27 (entire function) |

### 5.3 Why Hit Counts Are Inflated

The broad function-range entries appear at multiple PCs in the optimized
bytecode. When raptor records coverage, each PC hit within a broad range
increments the hit count for **every line** in that range.

For example, `s=238 l=167` (lines 13–17) appears at PCs 1, 2, 3, 4, 5, 9, 11,
29, and 30. After the optimizer restructures the function (inlining `add` and
`sub`), those PCs are hit more times during execution, driving line 13's hit
count up to 3.

### 5.4 Which Optimizer Steps Are Responsible

The Solidity compiler applies three levels of optimization. The relevant level
is the **Yul-based optimizer**, whose default step sequence includes:

| Step                          | Letter | Effect on source maps                                          |
| ----------------------------- | ------ | -------------------------------------------------------------- |
| **FullInliner**               | `i`    | Inlines function bodies into callers when heuristically        |
|                               |        | beneficial. Eliminates `return` statements of inlined          |
|                               |        | functions because return values are used directly in the       |
|                               |        | caller.                                                        |
| **ExpressionInliner**         | `e`    | Inlines simple expression-like functions. Can also eliminate   |
|                               |        | return values.                                                 |
| **RedundantAssignEliminator** | `r`    | Removes assignments that become dead after inlining. This can  |
|                               |        | strip return-related code that sourced from inlined functions. |
| **UnusedPruner**              | `u`    | Removes unused functions and variables. If an inlined          |
|                               |        | function's original definition is no longer referenced, it is  |
|                               |        | dropped entirely along with its source map entries.            |
| **StructuralSimplifier**      | `t`    | Restructures control flow (`if`/`switch`/`for`). Can merge     |
|                               |        | function epilogue code into broader ranges.                    |
| **ControlFlowSimplifier**     | `n`    | Simplifies control flow patterns, potentially merging the      |
|                               |        | return path into the function body range.                      |

The primary cause is the **FullInliner** (`i`). When `add` and `sub` are inlined
into `addAndSub`, the return values are consumed directly in the caller. The
explicit `return` instructions become dead code and are eliminated. Their source
map entries disappear with them.

Even `addAndSub`'s own `return result` (line 16) loses its dedicated entries
because the inlined code restructures the function epilogue. The
`StructuralSimplifier` and `ControlFlowSimplifier` merge the return path into
the broader function-range entry.

### 5.5 Why `FNDA` Shows `add` Called Twice

`add` is not fully inlined away — it still exists as a separate code section
with its own source map entries (12 entries in optimized vs 13 in unoptimized).
However, the optimizer created an additional inlined instance of `add`'s body
inside `addAndSub`. The jump-into marker (`j='i'`) for `add` is hit twice:

1. Once via the original function definition.
2. Once via the inlined copy inside `addAndSub`.

Because `FNDA` counts function entry points, it registers `add` as entered
twice.

## 6. Why This Is a Compiler-Level Limitation

The Solidity optimizer is designed to produce efficient bytecode, not to
preserve source mapping fidelity. When the compiler documentation describes the
optimizer, it explicitly warns:

> function inlining is an operation that can cause much bigger code, but it is
> often done because it results in opportunities for more simplifications.

And:

> when it comes to the Yul/intermediate-representation, there can be significant
> differences, for example, functions may be inlined, combined, or rewritten to
> eliminate redundancies

Source maps are updated to reflect the transformed code, but the granularity
changes: fine-grained per-statement entries are merged into coarse
function-level ranges. Lines that the optimizer eliminates (such as return
statements in inlined functions) lose their dedicated entries entirely.

This is not a bug in raptor. It is an inherent consequence of using source maps
to measure coverage of optimized bytecode. The same behavior occurs in other
coverage tools (e.g., `forge coverage`).

## 7. Mitigation Strategies

### 7.1 Measure Coverage on Unoptimized Builds (Recommended)

The standard approach in the Solidity ecosystem is to measure coverage against
unoptimized bytecode. This guarantees 1:1 source map fidelity. Raptor already
supports this via the `coverage-report-optimizer-disabled` fixture.

### 7.2 Customize the Yul Optimizer Sequence

The user can override the optimizer step sequence to exclude the inliner while
keeping other optimizations:

```bash
solc --optimize --yul-optimizations 'dhfoD[xarrscLMcCTU]uljmul:fDnTOcmu'
```

Removing `i` (FullInliner) from the sequence disables function inlining. The
tradeoff is larger bytecode and higher gas costs.

### 7.3 Accept Partial Coverage Fidelity

For optimized builds, accept that:

- Return statement lines in inlined internal functions may show 0 hits.
- Function entry counts may be inflated due to inlined copies.
- Lines at function boundaries may show elevated hit counts due to broad
  source-range reuse.

These discrepancies are predictable and bounded: they only affect return
statements and function signature lines within internal functions that the
optimizer decides to inline. External/public function boundaries are generally
preserved.

### 7.4 Post-Process Coverage Gaps

A post-processing step could infer that lines within a hit function's range were
executed, even when they lack dedicated source map entries. For example: if a
function's broad-range entry is hit and all non-return lines within the function
show hits, the return line can be assumed hit. This is heuristic and fragile,
but it could close the gap for simple cases like `TargetContractBasic`.

## 8. Key Files

| File                                                            | Role                                         |
| --------------------------------------------------------------- | -------------------------------------------- |
| `fixtures/coverage-report-optimizer-disabled/`                  | Fixture with `optimizer = false`             |
| `fixtures/coverage-report-optimizer-enabled/`                   | Fixture with `optimizer = true, runs = 1000` |
| `src/evm/coverage/reporter.rs`                                  | Coverage report generation and tests         |
| `src/evm/coverage/source_map.rs`                                | Source map parser                            |
| `external/argotorg/solidity/docs/internals/optimizer.rst`       | Solidity optimizer documentation             |
| `external/argotorg/solidity/docs/internals/source_mappings.rst` | Solidity source mapping documentation        |
