# Glossary

Consistent vocabulary for ripfuzz users and contributors.

## Core Terms

### Fuzzing Campaign

A single invocation of `ripfuzz run`. A campaign initializes the
**handler contract**, builds seed inputs, and orchestrates one or more
**fuzzers** that generate sequences of **function calls**, execute them against
a cloned contract state, and check that all **properties** still hold. If a
**failed assertion** is found, the campaign spawns one or more **shrinkers** to
minimize the failing sequence before reporting the result. Also called a "fuzz
run" or "test run".

### Handler Contract

The Solidity file you pass to `ripfuzz run` (e.g.
`./test/CounterHandler.sol:CounterHandler`). It is the contract ripfuzz
compiles, deploys, and exercises.

### Invariant Function

A Solidity function that encodes an invariant. By default it must:

- start with the prefix `invariant_`
- take no arguments
- be `pure` or `view`

Ripfuzz appends every invariant to the end of each function call sequence and
executes it in the same EVM loop. If an invariant reverts with a Solidity
`assert` failure (`Panic(0x01)`), the fuzzer records a failed assertion. The
return value, if any, is ignored. Synonyms: **invariant**, **property test**.

### Function-Level Invariant

A property that arises from the execution of a **specific function**. It
describes what must be true *before* and *after* that single function runs. For
example, after calling `deposit(uint256 amount)`, the contract's ETH balance
should increase by `amount` and the sender's balance should decrease by the same
amount.

### System-Level Invariant

A property that must hold true across the **entire execution** of a system,
regardless of which functions are called. These are more general than
function-level invariants. For example, the `xy = k` constant product formula
must always hold for a Uniswap pool, or the total deposited amount in a lending
protocol must never exceed `MAX_DEPOSIT_AMOUNT`.

### Handler Function

Any external or public function in the handler contract that is *not* a setup or
invariant function. Ripfuzz calls these with randomly-generated arguments to
mutate contract state. A single fuzz input is a **sequence of function calls**.
Synonyms: **function call**, **target function** (Foundry, Echidna).

### Setup Function

A function that establishes the initial state cloned for every fuzz input. The
contract **constructor** always runs once at deployment. If a function named
`setup()` exists, ripfuzz calls it once after deployment.

### Fuzzer

A single parallel fuzzing instance that executes function call sequences against
a cloned contract state and reports new coverage or failed assertions to the
campaign manager. By default ripfuzz spawns one fuzzer per available CPU core.

### Campaign Result

The aggregated output of a fuzzing campaign, including the total number of
iterations executed across all fuzzers and any failed assertions (assert panics)
discovered.

### Failed Assertion

A failure recorded when any call (handler function or invariant) reverts with a
Solidity `assert` panic (`Panic(0x01)`). The fuzzer treats a failed assertion as
a bug and adds it to the set of objectives. Reverts caused by `require` or other
reasons do not produce a failed assertion. Synonyms: **objective**, **bug**.

### Shrinker

A per-thread worker that minimizes a failing corpus item after a failed
assertion is discovered. The shrinker draws mutated copies of the current
smallest failing sequence, executes each on a fresh chain clone, and replaces
the shared item if the mutated sequence is still failing and strictly smaller.
The goal is to produce a minimal reproduction that triggers the same assertion
panic with the fewest possible calls.

## Coverage Terms

### Coverage-Guided Fuzzing

The technique that steers the fuzzer toward unexplored code by observing which
EVM instructions each input exercises. After every execution, ripfuzz compares
the coverage against all previously seen coverage. If the input reached a new
instruction, branch, call depth, or revert path, it is considered
**interesting** and added to the **corpus** for future mutation.

### Coverage Map

A data structure that records which parts of EVM bytecode were executed during a
fuzzing campaign. Ripfuzz maintains two kinds:

- **Local Coverage**: collected by the `Inspector` during a single execution of
  a call sequence. Reset for every sequence.
- **Global Coverage** (Shared Coverage): the merged union of all local coverage
  maps across every fuzzer thread. This is what the fuzzer checks to detect
  novelty.

### Coverage Update

The result of merging a local coverage map into the global map. A
`CoverageUpdate` counts how many genuinely new coverage points were discovered,
across six dimensions:

| Dimension           | Meaning                                      |
| ------------------- | -------------------------------------------- |
| `new_edges`         | A PC was executed for the first time         |
| `new_features`      | A PC was hit more deeply (higher AFL bucket) |
| `new_depths`        | A PC was hit at a new call-stack depth       |
| `new_reverts`       | A new revert path was exercised              |
| `new_jump_edges`    | A new JUMP/JUMPI destination was taken       |
| `new_jump_features` | A known jump edge was taken more times       |

If any of these counts is non-zero, the input is **interesting**.

### AFL Bucket

A coarse-grained classification of raw hit counts, borrowed from AFL. Ripfuzz
buckets raw hit counts into power-of-two buckets so that "hit 5 times" and "hit
6 times" are treated as the same coverage, while "hit 7 times" and "hit 8 times"
are treated as different (the loop crossed a threshold).

| Raw hits | Bucket |
| -------- | ------ |
| 0        | 0      |
| 1        | 1      |
| 2        | 2      |
| 3        | 4      |
| 4-7      | 8      |
| 8-15     | 16     |
| 16-31    | 32     |
| 32-127   | 64     |
| 128-255  | 128    |

### Jump Edge

A coverage signal that records the source and destination PCs of a `JUMP` or
`JUMPI` instruction. Two inputs that both reach a branch but take different
directions produce different jump edges. Encoded as a 64-bit marker:
`rotate_left(src_pc, 32) ^ dst_pc`.

### Source Map / Source Coverage

The mapping from EVM bytecode positions back to Solidity source code lines,
branches, and functions. Used only at the end of a campaign to produce a
human-readable coverage report. The fuzzing loop itself operates on raw bytecode
coverage for speed.

## Correspondence with Other Fuzzers

| Ripfuzz          | Foundry (invariant) | Medusa        | Echidna       |
| ---------------- | ------------------- | ------------- | ------------- |
| Handler Contract | Handler             | Target        | Target        |
| `invariant_`     | `invariant_`        | `property_`   | `echidna_`    |
| Handler Function | Handler function    | Function call | Function call |
| Campaign         | Test run            | Fuzzing run   | Test run      |
