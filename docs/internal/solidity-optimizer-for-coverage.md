# Solidity Optimizer and Source Coverage

This document explains how the Solidity optimizer interacts with source maps,
how the compiler preserves enough information for accurate coverage tracking
when optimization is enabled, and how raptor uses that fact to run fast,
optimizer-enabled fuzzing campaigns while still producing accurate `lcov`
reports.

## 1. How the Optimizer Affects the Source Map

### What the source map represents

The Solidity compiler emits a **source map** alongside the bytecode. It is a
semicolon-separated list where each element corresponds to one instruction
(offset by instruction index, not byte offset). The format is:

```
s:l:f:j:m
```

| Field | Meaning                                                      |
| ----- | ------------------------------------------------------------ |
| `s`   | Byte offset in the source file                               |
| `l`   | Length of the source range in bytes                          |
| `f`   | Source file index                                            |
| `j`   | Jump type (`i` = into function, `o` = return, `-` = regular) |
| `m`   | Modifier depth                                               |

Missing fields reuse the previous element's value. The mapping is built during
code generation and is updated whenever the compiler transforms the code.

### Three optimization levels (in order of execution)

1. **Codegen-level optimizations** — direct analysis of Solidity code before IR
   emission. In the legacy pipeline this is mostly implicit; in the IR pipeline
   (`--via-ir`) it is limited to cases that are hard to express in Yul but easy
   with high-level information (e.g., unchecked loop increments).

2. **Yul-based optimizer** — transforms Yul IR. This is the most powerful stage
   because it can reason across function calls. It inlines functions, eliminates
   common sub-expressions, removes dead code, reorders independent calls, and
   runs SSA-based simplifications.

3. **Opcode-based optimizer** — operates on EVM assembly. It splits code into
   basic blocks at `JUMP` and `JUMPDEST`, re-generates each block from a
   dependency graph, and drops unused operations. It also performs peephole
   simplifications and simple inlining.

### Concrete effects on the source map

| Optimization                         | Effect on source map                                                                                                                                                                                                                                                                                                             |
| ------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Dead code elimination**            | Instructions that are removed disappear from the bytecode, so their source-map entries also disappear. The surviving instructions keep their original mappings.                                                                                                                                                                  |
| **Function inlining**                | The body of the callee is copied into the caller. The inlined instructions usually carry the **callee's** source range, so the source map now contains a mix of caller and callee locations inside what was originally a single call site.                                                                                       |
| **Code reordering**                  | The Yul optimizer can reorder independent statements or hoist loop invariants. The source map follows the new instruction order, so consecutive PCs may jump between unrelated source locations.                                                                                                                                 |
| **Common subexpression elimination** | Duplicate expressions are replaced by a single evaluation. The reused result may be mapped to the original evaluation site, while the eliminated duplicate disappears.                                                                                                                                                           |
| **Generated / internal code**        | When the optimizer or codegen produces code that does not correspond to any user-written Solidity (e.g., ABI encoder v2, inline assembly wrappers), the compiler assigns it to **generated sources** with their own source IDs. These appear in `generatedSources` and are referenced by the source map with the new file index. |

### The `verbatim` caveat

The Solidity documentation explicitly notes that when the `verbatim` builtin is
used, the source mappings become invalid because the builtin is treated as a
single instruction even though it may expand to multiple opcodes.

### Generated sources

The compiler can produce "internal" source files that are not part of the
original input but are referenced from source mappings. These are exposed in the
output as:

```json
{
    "contracts": {
        "Source.sol": {
            "ContractName": {
                "evm": {
                    "bytecode": {
                        "generatedSources": [
                            {
                                "id": 2,
                                "name": "#utility.yul",
                                "language": "Yul",
                                "contents": "..."
                            }
                        ]
                    }
                }
            }
        }
    }
}
```

If an instruction has no associated source file, the source map uses index `-1`.

## 2. How the Compiler Preserves Info for Accurate Coverage Tracking

Despite the transformations above, the Solidity compiler does not discard the
information needed to map optimized bytecode back to the original source. It
preserves it through several mechanisms:

### Source maps are still generated for optimized code

The compiler emits `sourceMap` strings for both `bytecode` and
`deployedBytecode` even when `--optimize` is enabled. The mappings may be less
granular (a single source range may cover many instructions) and may reference
generated sources, but they are present and valid.

### The AST is always available

The compiler output includes the full AST (`ast`) for every source file. The AST
nodes carry their own source ranges (`src`). Because AST generation happens
before optimization, these ranges faithfully reflect the original Solidity
structure. A coverage tool can cross-reference the AST with the bytecode source
map to determine which statements, branches, and functions are covered.

### Debug info output

The compiler can include extra debug information via the `debugInfo` setting:

```json
{
    "settings": {
        "debug": {
            "debugInfo": ["location", "snippet", "ast-id", "ethdebug"]
        }
    }
}
```

- `location` — injects source location comments into the produced EVM assembly.
- `snippet` — quotes the source snippet in the comment.
- `ast-id` — annotates elements with the original Solidity AST node ID.
- `ethdebug` — experimental structured debug format.

These annotations help tools correlate optimized bytecode with original source
constructs even when the optimizer has heavily rewritten the code.

### IR pipeline preserves structure better

When compiling via IR (`--via-ir`), the compiler produces Yul IR that **closely
matches the structure of the Solidity code**. Nearly all optimizations are
deferred to the Yul optimizer module. This means:

- The initial IR has source mappings that are very close to the original
  Solidity.
- The Yul optimizer then transforms the IR, but because the starting point is
  structurally similar to the source, the final source maps tend to be more
  accurate than those produced by the legacy pipeline.
- In the legacy pipeline, bytecode is generated immediately and the optimizer
  works at the opcode level, which can obscure the connection to the source.

### `functionDebugData` is preserved

The artifact output contains `functionDebugData`, which maps function names to
their entry point PC, parameter slots, and return slots. This is preserved even
with optimization enabled and helps coverage tools identify function boundaries
in the bytecode.
