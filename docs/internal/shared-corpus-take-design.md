# `SharedCorpus::take` Design

This document explains how
[`SharedCorpus::take`](../../src/fuzzer/corpus/mod.rs) produces the next fuzz
input, how that input is structured, and how the random value generation
strategies documented in
[`random-uint-generation.md`](random-uint-generation.md) fit into the pipeline.

## Overview

`take()` is the **sole entry point** through which a fuzzer worker obtains an
input to execute. It has two mutually exclusive paths:

```
┌─────────────────────────────────────────────────────────────┐
│                    SharedCorpus::take()                     │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│   ┌─────────────┐      ┌─────────────────────────────┐   │
│   │ Corpus Hit  │      │         Corpus Miss         │   │
│   │ (items > 0) │      │      (items == 0)           │   │
│   └──────┬──────┘      └──────────────┬──────────────┘   │
│          │                             │                  │
│   ┌──────▼──────┐      ┌───────────────▼──────────────┐   │
│   │ Random pick │      │ generate_random_sequence()   │   │
│   │ from map    │      │                              │   │
│   │             │      │  1. Pick random function     │   │
│   │  → clone()  │      │  2. For each arg:            │   │
│   │             │      │     random_dyn_value()       │   │
│   │             │      │     → random::uint() / int() │   │
│   │             │      │  3. Build Call + Item        │   │
│   └──────┬──────┘      └───────────────┬──────────────┘   │
│          │                             │                  │
│          └─────────────┬───────────────┘                  │
│                        │                                  │
│                   ┌────▼────┐                              │
│                   │  Item   │                              │
│                   └─────────┘                              │
└─────────────────────────────────────────────────────────────┘
```

## Path A — Corpus Hit

When the corpus contains at least one item:

1. Snapshot all items into a `Vec<Item>`.
2. Atomically increment a `seed: AtomicU64` counter.
3. Seed a `fastrand::Rng` with that counter.
4. Pick a uniformly random index.
5. Return a **clone** of the selected item.

### Current behaviour

The clone is returned **verbatim**. No per-argument mutation is applied. This
means a corpus hit is an exact replay of a previously discovered sequence.

### How other fuzzers differ

| Fuzzer      | Corpus hit behaviour                                                                                                                         |
| ----------- | -------------------------------------------------------------------------------------------------------------------------------------------- |
| **Medusa**  | Picks a corpus sequence, then mutates it with configurable probability (sequence-level splice/interleave/head/tail + per-argument mutation). |
| **Echidna** | Picks a corpus call (`genAbiCallM`), then applies `mutateAbiCall` which mutates exactly one argument.                                        |
| **Foundry** | Proptest's `ValueTree` generates "similar" values automatically; corpus items are part of the strategy exploration.                          |

### Gap in Raptor

Raptor has **no mutation on corpus hits**. Every replay is identical. This means
the fuzzer cannot explore the neighbourhood of an interesting sequence without
first discovering a brand-new sequence that happens to land nearby.

## Ideal Design (Production-Grade)

Based on Medusa, Echidna, and Foundry, an ideal `take()` pipeline would look
like this:

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                         Ideal SharedCorpus::take()                          │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│  Top-level distribution (corpus may or may not be empty)                    │
│                                                                             │
│  ┌──────────────────────────┐  ┌────────────────────────────────────────┐   │
│  │  30 % Fresh Sequence     │  │  70 % Corpus-Based Sequence            │   │
│  │  (generate from scratch) │  │  (pick existing + mutate)              │   │
│  └────────────┬─────────────┘  └────────────────────┬───────────────────┘   │
│               │                                     │                       │
│   ┌───────────▼───────────────┐   ┌─────────────────▼──────────────────┐    │
│   │ generate_fresh_sequence() │   │   weighted_sequence_mutation()     │    │
│   │                           │   │                                    │    │
│   │  1. Random length         │   │  ┌────┐ ┌────┐ ┌────┐  ┌────┐      │    │
│   │  2. Random function       │   │  │Head│ │Tail│ │Splice │ │Inter│   │    │
│   │  3. For each arg:         │   │  └────┘ └────┘ └────┘  └────┘      │    │
│   │     random_dyn_value()    │   │  ┌─────────────────────────────┐   │    │
│   │     ┌────────────────┐    │   │  │ 10 % Mutated variants       │   │    │
│   │     │ Value Sources  │    │   │  │ (per-arg mutation applied)  │   │    │
│   │     │ ┌──┐┌──┐┌──┐   │    │   │  └─────────────────────────────┘   │    │
│   │     │ │35││25││20│   │    │   │  ┌─────────────────────────────┐   │    │
│   │     │ │% ││% ││% │   │    │   │  │ 90 % Unmodified variants    │   │    │
│   │     │ │PR││ED││DL│   │    │   │  │ (verbatim replay)           │   │    │
│   │     │ └──┘└──┘└──┘   │    │   │  └─────────────────────────────┘   │    │
│   │     │ ┌──┐┌──┐┌──┐   │    │   │                                    │    │
│   │     │ │15││ 5││      │    │   │  Weighted split (of the 70 %):     │    │
│   │     │ │% ││% ││      │    │   │  ┌──────────────────────────────┐  │    │
│   │     │ │EC││FX││      │    │   │  │ Unmodified head   ~61 %      │  │    │
│   │     │ └──┘└──┘└──┘   │    │   │  │ Unmodified splice ~15 %      │  │    │
│   │     └────────────────┘    │   │  │ Unmodified tail    ~8 %      │  │    │
│   │                           │   │  │ Unmodified inter   ~8 %      │  │    │
│   │  PR = Pure Random (biased)│   │  │ Mutated head       ~6 %      │  │    │
│   │  ED = Edge Cases          │   │  │ Mutated splice     ~2 %      │  │    │
│   │  DL = Dynamic Dictionary  │   │  │ Mutated tail       ~1 %      │  │    │
│   │  EC = AST Literals        │   │  │ Mutated inter      ~1 %      │  │    │
│   │  FX = Fixtures            │   │  └──────────────────────────────┘  │    │
│   └───────────┬───────────────┘   └─────────────────┬──────────────────┘    │
│               │                                     │                       │
│               └─────────────────┬───────────────────┘                       │
│                                 │                                           │
│                          ┌──────▼──────┐                                    │
│                          │    Item     │                                    │
│                          └──────┬──────┘                                    │
│                                 │                                           │
│                    ┌────────────▼────────────┐                              │
│                    │   Shrink (on crash)     │                              │
│                    │                         │                              │
│                    │  1. Remove calls        │                              │
│                    │  2. Simplify args       │                              │
│                    │     ┌────┐  ┌────┐      │                              │
│                    │     │÷2  │  │→0  │      │                              │
│                    │     └────┘  └────┘      │                              │
│                    └─────────────────────────┘                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Why the corpus is never "locked"

Even when `items.len() > 0`, a production fuzzer **always** reserves a non-zero
probability for generating fresh sequences from scratch. Medusa uses
`NewSequenceProbability = 0.3` (30 %). Echidna has a similar `pSynthA` parameter
that splits between dictionary-based and synthesised generation.

If the fuzzer always replayed or mutated existing items, it could never:

- discover entirely new function combinations,
- escape local maxima in coverage, or
- exercise functions that happen to be absent from the current corpus.

### Detailed distribution breakdown

#### Top level (sequence origin)

| Probability | Action             | Description                                                     | Source                                                                                                                                                |
| ----------- | ------------------ | --------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
| **30 %**    | **Fresh sequence** | Generate a brand-new call sequence from scratch.                | Medusa `NewSequenceProbability = 0.3` in `fuzzer.go` [`external/crytic/medusa/fuzzing/fuzzer.go:893`](../../external/crytic/medusa/fuzzing/fuzzer.go) |
| **70 %**    | **Corpus-based**   | Pick an existing corpus item and apply sequence-level mutation. | Medusa complement of `NewSequenceProbability`                                                                                                         |

#### Corpus-based sub-distribution (Medusa-style weights)

The 70 % corpus path is further split by weighted operators. The weights below
are taken verbatim from Medusa's `defaultCallSequenceGeneratorConfigFunc`:

| Weight | ~Probability | Operator                  | Description                                              | Source                                                                                                            |
| ------ | ------------ | ------------------------- | -------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------- |
| 800    | 60.6 %       | **Unmodified head**       | Take the first N calls of an existing sequence verbatim. | Medusa `RandomUnmodifiedCorpusHeadWeight = 800` [`fuzzer.go:900`](../../external/crytic/medusa/fuzzing/fuzzer.go) |
| 200    | 15.2 %       | **Unmodified splice**     | Take a random sub-slice of an existing sequence.         | Medusa `RandomUnmodifiedSpliceAtRandomWeight = 200`                                                               |
| 100    | 7.6 %        | **Unmodified tail**       | Take the last N calls of an existing sequence.           | Medusa `RandomUnmodifiedCorpusTailWeight = 100`                                                                   |
| 100    | 7.6 %        | **Unmodified interleave** | Interleave calls from two existing sequences.            | Medusa `RandomUnmodifiedInterleaveAtRandomWeight = 100`                                                           |
| 80     | 6.1 %        | **Mutated head**          | Take the first N calls, then mutate each argument.       | Medusa `RandomMutatedCorpusHeadWeight = 80`                                                                       |
| 20     | 1.5 %        | **Mutated splice**        | Take a random sub-slice, then mutate each argument.      | Medusa `RandomMutatedSpliceAtRandomWeight = 20`                                                                   |
| 10     | 0.8 %        | **Mutated tail**          | Take the last N calls, then mutate each argument.        | Medusa `RandomMutatedCorpusTailWeight = 10`                                                                       |
| 10     | 0.8 %        | **Mutated interleave**    | Interleave two sequences, then mutate each argument.     | Medusa `RandomMutatedInterleaveAtRandomWeight = 10`                                                               |

Total weight = 1 320. Mutated variants sum to 120 / 1 320 ≈ 9.1 % (rounded to 10
% in the diagram).

#### Per-argument mutation (when a mutated variant is selected)

For each leaf value in the sequence:

| Probability | Mutation        | Description                                | Source                                                                                                    |
| ----------- | --------------- | ------------------------------------------ | --------------------------------------------------------------------------------------------------------- |
| **10 %**    | **Mutate**      | Apply one of the mutation operators below. | Medusa `MutateIntegerProbability = 0.1` [`fuzzer.go:910`](../../external/crytic/medusa/fuzzing/fuzzer.go) |
| **90 %**    | **Leave as-is** | The argument is replayed verbatim.         | Medusa complement of `MutateIntegerProbability`                                                           |

When mutation is triggered, the operator is chosen uniformly from the pooled
operators below:

| Operator                    | Description                                                                                              | Source                                                                                                                                      |
| --------------------------- | -------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------- |
| Increment / Decrement       | `±1` (wrapping).                                                                                         | Foundry `IncrementDecrementMutator` [`mutators.rs:52`](../../external/foundry-rs/foundry/crates/evm/fuzz/src/strategies/mutators.rs)        |
| Bit flip                    | Flip one random bit.                                                                                     | Foundry `BitMutator` [`mutators.rs:201`](../../external/foundry-rs/foundry/crates/evm/fuzz/src/strategies/mutators.rs)                      |
| AFL interesting word        | Replace with a known interesting value (`INTERESTING_8/16/32`).                                          | Foundry `InterestingWordMutator` [`mutators.rs:244`](../../external/foundry-rs/foundry/crates/evm/fuzz/src/strategies/mutators.rs) + LibAFL |
| Gaussian noise              | Scale the value by a random Gaussian-like factor (σ multipliers: `0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0`). | Foundry `GaussianNoiseMutator` [`mutators.rs:99`](../../external/foundry-rs/foundry/crates/evm/fuzz/src/strategies/mutators.rs)             |
| Arithmetic (Medusa-style)   | Add / subtract / multiply / divide / modulo by another random dictionary value.                          | Medusa `integerMutationMethods` [`generator_mutational.go`](../../external/crytic/medusa/fuzzing/valuegeneration/generator_mutational.go)   |
| `mutateNum` (Echidna-style) | `x ± uniform[0, 2x]`.                                                                                    | Echidna `mutateNum` [`ABI.hs`](../../external/crytic/echidna/lib/Echidna/ABI.hs)                                                            |

#### Fresh sequence — value source distribution

For every argument generated from scratch. These percentages are a **synthesis**
of all three fuzzers rather than a direct copy of any single one:

| Probability | Source                   | Description                                                                                                | Rationale                                                                                                                                                                                                                                                         |
| ----------- | ------------------------ | ---------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **35 %**    | **Pure Random (biased)** | Echidna-style distribution: 76 % near-full-range, 9.5 % small `[0,1023]`, 9.5 % near-max, 4.8 % power-law. | Echidna `getRandomUint` [`ABI.hs`](../../external/crytic/echidna/lib/Echidna/ABI.hs). We elevate its weight because biased random finds more bugs than uniform.                                                                                                   |
| **25 %**    | **AST Literals**         | Number literals extracted from the Solidity AST, filtered by type width.                                   | Foundry `LiteralMaps` [`literals.rs`](../../external/foundry-rs/foundry/crates/evm/fuzz/src/strategies/literals.rs) uses 40–50 % for fixtures / typed samples. We allocate 25 % to AST literals alone.                                                            |
| **20 %**    | **Dynamic Dictionary**   | Values observed at runtime: return values, storage slots, comparison operands, `keccak256` inputs.         | Foundry `fuzz_param_from_state` [`param.rs`](../../external/foundry-rs/foundry/crates/evm/fuzz/src/strategies/param.rs) splits 50/50 between typed samples and general state values. Echidna also scrapes return values into `dictValues`.                        |
| **15 %**    | **Edge Cases**           | `0`, `1`, `max`, `max-1`, `max-2`, `max-3` (or signed equivalents).                                        | Foundry `UintStrategy` gives edge cases 10 % weight [`uint.rs`](../../external/foundry-rs/foundry/crates/evm/fuzz/src/strategies/uint.rs). We raise to 15 % because edge cases are cheap and high-impact.                                                         |
| **5 %**     | **Fixtures**             | User-defined `fixture_<name>()` arrays from the target contract.                                           | Foundry `UintStrategy` gives fixtures 40 % weight when fixtures exist [`uint.rs`](../../external/foundry-rs/foundry/crates/evm/fuzz/src/strategies/uint.rs). We lower to 5 % because fixtures are optional; when absent this weight can roll over to pure random. |

### Key differences from current Raptor

| Feature                   | Current Raptor     | Ideal Design                                                                 | Source Fuzzer              |
| ------------------------- | ------------------ | ---------------------------------------------------------------------------- | -------------------------- |
| **Sequence mutation**     | none               | weighted head / tail / splice / interleave / no-mutation                     | Medusa                     |
| **Per-argument mutation** | none               | 10 % per leaf; inc/dec, bit-flip, AFL interesting words                      | Echidna + Foundry          |
| **Value source mix**      | AST literals only  | AST literals + edge cases + pure random + dynamic dictionary                 | All three                  |
| **Dynamic dictionary**    | none               | collect return values, storage slots, `SLOAD`/`SSTORE`, `keccak256` operands | Foundry + Echidna          |
| **Per-type literals**     | flat `Vec<String>` | `HashMap<bit_width, Vec<String>>`                                            | Foundry                    |
| **Shrinking**             | none               | remove calls + bisect uints / drive ints toward zero                         | Medusa + Echidna + Foundry |
| **Fixture support**       | none               | user-defined `fixture_<name>()` functions                                    | Foundry                    |

## Path B — Corpus Miss

When the corpus is empty (early campaign or after reset), `take()` generates a
fresh sequence from scratch.

### Step 1 — `generate_random_sequence()`

```rust
fn generate_random_sequence(&self, rng: &mut fastrand::Rng) -> Vec<Call> {
    let len = rng.usize(1..=max_calls_length);
    for _ in 0..len {
        let func = &functions[rng.usize(0..functions.len())];
        let values: Vec<DynSolValue> = func
            .inputs
            .iter()
            .filter_map(|p| p.resolve().ok())
            .map(|ty| random_dyn_value(&ty, rng, &self.inner.literals))
            .collect();
        calls.push(Call { function: func.clone(), args: Tuple(values), ..Default::default() });
    }
    calls
}
```

Key properties:

- **Sequence length**: uniform `[1, max_calls_length]`.
- **Function selection**: uniform across all target functions.
- **Argument independence**: each argument is generated independently; there is
  no cross-argument constraint solving.
- `caller` and `value` (wei) are always `DEFAULT_DEPLOYER` and `0`.

### Step 2 — `random_dyn_value()`

This is the **type-driven value dispatcher**. It matches on `DynSolType` and
delegates to the appropriate random generator:

| Solidity type | Generator              | Value sources                                      |
| ------------- | ---------------------- | -------------------------------------------------- |
| `bool`        | inline                 | literal (40 % bias) → `rng.bool()`                 |
| `uintN`       | `random::uint(N, ...)` | literal (40 %) → edge cases (30 %) → random (30 %) |
| `intN`        | `random::int(N, ...)`  | literal (40 %) → edge cases (30 %) → random (30 %) |
| `address`     | inline                 | hex literal → number literal → random bytes        |
| `bytesN`      | inline                 | hex literal → random bytes                         |
| `bytes`       | inline                 | hex literal → random length + random bytes         |
| `string`      | inline                 | literal → random length + random chars             |
| `function`    | inline                 | random 24 bytes                                    |
| array / tuple | recursive              | `random_dyn_value(inner, ...)`                     |

### Connection to `random-uint-generation.md`

The `uint` and `int` arms are the only ones that currently implement the
three-phase model (literal → edge case → random) described in
[`random-uint-generation.md`](random-uint-generation.md). All other types still
use a simpler literal-or-random two-phase model.

This creates an **asymmetry**: `uint256` gets edge-case bias and per-type
masking, but `address`, `string`, and `bytes` do not.

## Data Model

Understanding `take()` requires understanding what an `Item` actually contains.

```
Item
└── calls: Vec<Call>
    └── Call
        ├── function: Function          // ABI definition (selector, inputs, mutability)
        ├── args: DynSolValue::Tuple     // Concrete argument values
        ├── value: U256                 // Wei sent with call
        └── caller: Address             // Transaction sender
```

### Content hashing

`Item::id()` and `Call::content_hash()` compute a Keccak256 hash over the
**execution-relevant** fields (`caller`, `value`, `calldata`). This means:

- Two items with different human-readable metadata but identical calldata hash
  to the same ID.
- The ID is stable across serialisation round-trips.

This design is borrowed from Medusa and is what allows deduplication in
`SharedCorpus::add()`.

## The Full Pipeline from `take()` to EVM Execution

```
SharedCorpus::take()
  ├── returns Item
  │      └── calls: Vec<Call>
  │            └── each Call has args: DynSolValue::Tuple
  │
  └── Fuzzer worker converts Item to ExecInput
         └── for each Call:
                Call::into_transaction(target)
                └── chain::Transaction
                       └── EVM executor runs it
```

The worker receives an `Item`, converts each `Call` into a `Transaction`, and
feeds those transactions into the EVM loop. After the sequence finishes, the
worker checks coverage and decides whether to `SharedCorpus::add()` the item.

## Design Decisions and Trade-offs

### 1. Atomic seed counter

`take()` seeds its RNG from an `AtomicU64` that increments on every call. This
makes the sequence of `take()` outputs deterministic for a given start seed, but
it also means all fuzzer workers share the same linear seed space.

**Trade-off**: simple and lock-free, but workers can theoretically see
overlapping seeds if the counter wraps (unlikely for a 64-bit value).

### 2. No per-argument mutation on corpus hits

`take()` returns a verbatim clone when the corpus is non-empty. This is
intentionally simple, but it means Raptor does not have a "mutate existing
corpus item" phase that Medusa and Echidna both implement.

**Trade-off**: less code complexity, but slower exploration of the state space
around interesting sequences.

### 3. Flat literal dictionary

`ExtractedLiterals` stores all number literals in a single `Vec<String>`
regardless of their original type. `random::uint` filters at generation time
(`u <= max`), but a `uint8` parameter can still receive a `uint256` max literal
40 % of the time if it happens to be the randomly chosen literal.

**Trade-off**: simple extraction code, but less precise type matching than
Foundry's per-type `LiteralMaps`.

### 4. No dynamic dictionary

Only AST literals are used. Return values, storage slots, and comparison
operands from EVM execution are not collected and fed back into the generator.

**Trade-off**: no runtime overhead for dictionary maintenance, but the fuzzer
cannot adapt to values that matter at runtime.

### 5. No shrinking

When a crash is found, the exact `Item` that triggered it is reported. There is
no attempt to minimise the sequence length or individual argument values.

**Trade-off**: simpler bug reporting, but crashes may contain irrelevant calls
or unnecessarily large values.

## Comparison with Other Fuzzers

| Dimension                               | Raptor                           | Medusa                                     | Echidna                        | Foundry                           |
| --------------------------------------- | -------------------------------- | ------------------------------------------ | ------------------------------ | --------------------------------- |
| **take() output when corpus non-empty** | verbatim clone                   | mutated corpus sequence                    | mutated corpus call            | proptest `ValueTree` walk         |
| **Sequence generation**                 | uniform length, uniform function | weighted splice / interleave / head / tail | one call at a time, dict-based | proptest strategy composition     |
| **Per-argument mutation**               | none                             | 10 % chance, arithmetic                    | 10 % chance, `±[0,2x]`         | proptest mutator pool             |
| **Dynamic dictionary**                  | none                             | return values                              | return values                  | storage, logs, cmp, return values |
| **Shrinking**                           | none                             | `ShrinkingValueMutator`                    | `shrinkAbiCall`                | `UintValueTree` + proptest        |
| **Literal source**                      | AST only                         | AST only                                   | AST + runtime                  | AST + runtime + fixtures          |

## Recommended Evolution

Based on the gaps identified above, the most impactful changes to `take()` and
its pipeline would be:

1. **Add per-argument mutation on corpus hits**. When returning a clone, walk
   the `DynSolValue` tree and apply a small mutation probability (e.g. 10 %) to
   each leaf. Reuse the existing `random::uint` / `random::int` generators for
   the mutation source.

2. **Add a dynamic dictionary**. During EVM execution, collect return values,
   `SSTORE` values, and `keccak256` operands. Add them to a `papaya::HashMap`
   keyed by `DynSolType`. Pass this map into `random_dyn_value` so it can draw
   from runtime-discovered values in addition to AST literals.

3. **Per-type literal buckets**. Change `ExtractedLiterals` from a flat
   `numbers: Vec<String>` to `numbers: HashMap<usize, Vec<String>>` keyed by bit
   width. This prevents a `uint8` parameter from receiving a `uint256` max
   literal.

4. **Sequence mutation operators**. Add Medusa-style splice and interleave
   operators: pick two corpus items and combine their call sequences. This can
   be done before `take()` returns the item.

5. **Shrinking pass**. When a crash is recorded, run a shrinking loop that tries
   to remove calls from the sequence and simplify individual arguments
   (binary-search bisection for uints, toward-zero for ints).

## References

- `SharedCorpus::take` — `src/fuzzer/corpus/mod.rs`
- `generate_random_sequence` — `src/fuzzer/corpus/mod.rs`
- `random_dyn_value` — `src/fuzzer/corpus/mod.rs`
- `random::uint` / `random::int` — `src/fuzzer/corpus/random/uint.rs` / `int.rs`
- `Item` — `src/fuzzer/corpus/item.rs`
- `Call` — `src/fuzzer/corpus/call.rs`
- `ExtractedLiterals` — `src/fuzzer/corpus/extractor.rs`
- Medusa sequence generator —
  `external/crytic/medusa/fuzzing/fuzzer_worker_sequence_generator.go`
- Echidna `genAbiCallM` — `external/crytic/echidna/lib/Echidna/ABI.hs`
- Foundry `fuzz_calldata_from_state` —
  `external/foundry-rs/foundry/crates/evm/fuzz/src/strategies/calldata.rs`
