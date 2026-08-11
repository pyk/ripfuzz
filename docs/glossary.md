# Glossary

Consistent vocabulary for ripfuzz users and contributors.

## Core Terms

### Fuzzing Campaign

A single invocation of `ripfuzz run`. The campaign deploys the **harness
contract** and coordinates **fuzzers** (and **shrinkers**, when a **failed
assertion** is found) until the run finishes with a **campaign result**. Also
called a "fuzz run" or "test run".

### Harness Contract

The Solidity file you pass to `ripfuzz run` (e.g. `Harness` or
`src/Harness.sol:Harness`). It is the contract ripfuzz compiles, deploys, and
fuzzes.

### Invariant Function

A Solidity function that checks an invariant. By default it must:

- start with the prefix `invariant_`
- take no arguments
- be `pure` or `view`

Ripfuzz appends every invariant to the end of each function call sequence and
executes it in the same EVM loop. If an invariant reverts with a Solidity
`assert` failure (`Panic(0x01)`), the fuzzer records a failed assertion. The
return value, if any, is ignored. Synonyms: **invariant**, **property test**.

### Max Function

A Solidity function whose `uint256` return value ripfuzz maximizes in **max
mode**. It must:

- start with the prefix `max_`
- take no arguments
- return a single `uint256`
- be `pure` or `view`

Ripfuzz calls the max function after each handler call in the sequence and
keeps the highest value plus the shortest prefix that produced it. Reverted or
empty results score `0`. A value above `0` is the finding. Synonyms:
**objective**, **optimization test** (Medusa).

### Max Mode

The campaign mode entered automatically when a harness declares a `max_*`
function. Max mode and **invariant mode** are mutually exclusive: ripfuzz
maximizes the `max_*` function and never runs `invariant_*` functions. Max mode
supports exactly one `max_*` function and rejects harnesses that also declare
`invariant_*` functions. Best sequences are shrunk while preserving their
value, reported with the maximum value, and written to the corpus for reuse.

### Function-Level Invariant

A property that arises from the execution of a **specific function**. It
describes what must be true *before* and *after* that single function runs. For
example, after calling `deposit(uint256 amount)`, the contract's ETH balance
should increase by `amount` and the sender's balance should decrease by the
same amount.

### Protocol-Level Invariant

A property that must hold after **any sequence of function calls**, not just
one specific function. These are more general than function-level invariants.
For example, the `xy = k` constant product formula must always hold for a
Uniswap pool, or the total deposited amount in a lending protocol must never
exceed `MAX_DEPOSIT_AMOUNT`.

### Handler Function

Any external or public function in the harness contract that is *not* a setup
or invariant or max function. Ripfuzz calls these with randomly-generated
arguments to mutate contract state. A single fuzz input is a **sequence of
function calls**. Synonyms: **function call**, **target function** (Foundry,
Echidna).

### Setup Function

A function that establishes the initial state cloned for every fuzz input. The
contract **constructor** always runs once at deployment. If a function named
`setup()` exists, ripfuzz calls it once after deployment.

### Fork

A remote chain snapshot selected with `rvm.fork(url, blockNumber)`. Campaigns
start as an empty sandbox; forking opts into on-chain state at a pinned block.
Multiple forks are cached by `(url, block)`. **Remote state is isolated per
fork**; **harness storage and other local accounts are shared across forks** so
you can track cross-chain properties (for example value conservation). See
[fork-mode.md](./fork-mode.md).

### Invariant Fuzzer

A single parallel fuzzing instance (the `InvariantFuzzer`) that executes
function call sequences against a cloned contract state and reports new
coverage or failed assertions to the campaign manager. By default ripfuzz
spawns one fuzzer per available CPU core.

### Maxxing Fuzzer

A single parallel fuzzing instance in **max mode** (the `MaxxingFuzzer`). It
executes handler calls followed by the `max_*` function call, merges coverage,
and records the highest value plus the shortest handler prefix that produced
it.

### Campaign Result

The aggregated output of a fuzzing campaign, including the total number of
iterations executed across all fuzzers and any failed assertions (assert
panics) discovered. Distinct failed assertions are deduplicated and each one is
minimized separately.

### Failed Assertion

A failure recorded when any call (handler function or invariant) reverts with a
Solidity `assert` panic (`Panic(0x01)`). The fuzzer treats a failed assertion
as a bug and adds it to the set of objectives. Reverts caused by `require` or
other reasons do not produce a failed assertion. Synonyms: **objective**,
**bug**.

### Shrinker

A per-thread worker that minimizes a failing corpus item after a failed
assertion is discovered. When a campaign collects multiple distinct failed
assertions, the shrinker minimizes each one independently. It draws mutated
copies of the current smallest failing sequence, executes each on a fresh chain
clone, and replaces the shared item if the mutated sequence is still failing
and strictly smaller. The goal is to produce a minimal reproduction that
triggers the same assertion panic with the fewest possible calls.

### Max Shrinker

A per-thread worker that minimizes the best sequence of a **max function**. It
draws mutated copies of the current best sequence, executes each followed by
the max function call, and accepts the candidate when it preserves or improves
the stored value and shrinks the sequence.

## Coverage Terms

### Coverage-Guided Fuzzing

The technique that steers the fuzzer toward unexplored code by observing which
EVM instructions each input exercises. After every execution, ripfuzz compares
the coverage against all previously seen coverage. If the input reached a new
instruction, branch, call depth, or revert path, it is considered
**interesting** and added to the **corpus** for future mutation.

### Coverage Map

A data structure that records which parts of EVM bytecode were executed during
a fuzzing campaign. Ripfuzz maintains two kinds:

- **Execution Coverage**: collected by the Inspector during a single execution
  of a call sequence. Reset for every sequence.
- **Shared Coverage**: the merged union of all execution coverage maps across
  every fuzzer thread. This is what the fuzzer checks to detect novelty.

### Coverage Update

The result of merging execution coverage into the shared map. A coverage update
counts how many genuinely new coverage points were discovered, across four
dimensions:

| Dimension        | Meaning                                |
| ---------------- | -------------------------------------- |
| `new_edges`      | A PC was executed for the first time   |
| `new_depths`     | A PC was hit at a new call-stack depth |
| `new_reverts`    | A new revert path was exercised        |
| `new_jump_edges` | A new JUMP/JUMPI destination was taken |

Coverage is binary for novelty: hitting the same PC or jump edge again does not
count as new, even if the hit count is higher. If any of these counts is
non-zero, the input is **interesting**.

### Jump Edge

A coverage signal that records the source and destination PCs of a taken `JUMP`
or `JUMPI`. Two inputs that reach the same branch but jump to different
destinations produce different jump edges. Encoded as a 64-bit marker:
`rotate_left(src_pc, 32) ^ dst_pc`.

### Source Map / Source Coverage

The mapping from EVM bytecode positions back to Solidity source ranges. After a
campaign, the coverage reporter uses source maps plus shared coverage to write
an `lcov.info` report with line and function hits. The fuzzing loop itself
operates on raw bytecode coverage for speed.

## In Other Fuzzers

| Ripfuzz          | Foundry (invariant) | Medusa        | Echidna       |
| ---------------- | ------------------- | ------------- | ------------- |
| Harness Contract | Handler             | Target        | Target        |
| `invariant_`     | `invariant_`        | `property_`   | `echidna_`    |
| Handler Function | Handler function    | Function call | Function call |
| Campaign         | Test run            | Fuzzing run   | Test run      |
