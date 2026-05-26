# Random `uint` Generation in EVM Fuzzers

This document analyses how Medusa, Echidna, and Foundry generate random unsigned
integers (`uint8` through `uint256`), structured along three orthogonal
dimensions:

1. **Phase** — whether the value is generated from scratch (fuzz), derived from
   an existing value (mutation), or simplified after a crash (shrink).
2. **Value Sources** — where candidate values come from (pure random, literals,
   edge cases, dynamic execution state, fixtures).
3. **Distribution** — the probability that each source is chosen during a given
   phase.

## Medusa

### Architecture

Medusa uses a **single `MutationalValueGenerator`** that wraps a
`RandomValueGenerator`. The mutational generator is both the _fuzz_ and
_mutation_ engine. A separate `ShrinkingValueMutator` handles _shrink_.

### 1. Phase — Fuzz (generate from scratch)

`MutationalValueGenerator.GenerateInteger(signed, bitLength)` delegates to
`mutateIntegerInternal(nil, signed, bitLength)`.

#### Value Sources

| Source                   | Description                                                                                                                                                |
| ------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Pure Random**          | `RandomValueGenerator.GenerateInteger` fills `bitLength/8` bytes with random data and clamps to `[min, max]`. Uniform across the full range.               |
| **Literals (AST)**       | `ValueSet` is seeded from AST `Literal` nodes (`kind="number"`). Hex literals and subdenominations (`wei`, `ether`, `seconds`, …) are parsed to `big.Int`. |
| **Hard-coded constants** | `ValueSet` is pre-seeded with `0`, `1`, `-1`, `2`.                                                                                                         |
| **Bounds**               | `min` and `max` for the target bit width are added to the input pool.                                                                                      |

#### Distribution

```
50 % → Pure Random  (GenerateRandomIntegerBias = 0.5)
50 % → Dictionary + Mutation
```

When the dictionary path is taken:

1. Pick a random base value from `ValueSet.Integers() + {min, max}`.
2. Perform `0` to `1` arithmetic mutation rounds (default config):
    - add / subtract / multiply / divide / modulo another random input.
3. Clamp to `[min, max]` after each round.

Because `MinMutationRounds` defaults to `0`, the base value itself is returned
~50 % of the time the dictionary path is entered.

### 2. Phase — Mutation (mutate an existing call)

`MutationalValueGenerator.MutateInteger(i, signed, bitLength)` is called when
mutating a corpus call sequence.

#### Distribution

```
90 % → No change  (MutateIntegerProbability = 0.1)
10 % → Mutate
   ├── 50 % → Replace with newly generated value  (MutateIntegerGenerateNewBias = 0.5)
   └── 50 % → Mutate existing value (arithmetic, same methods as fuzz phase)
```

### 3. Phase — Shrink (simplify a failing input)

`ShrinkingValueMutator.MutateInteger(i, signed, bitLength)` is invoked during
crash minimisation.

#### Value Sources

| Source          | Description                                                                                            |
| --------------- | ------------------------------------------------------------------------------------------------------ |
| **Divide by 2** | `integerShrinkingMethods[1]` — halves the value.                                                       |
| **Toward zero** | `integerShrinkingMethods[0]` — subtracts a random input for positive values, adds for negative values. |
| **Bounds**      | `min` and `max` for the bit width are part of the input pool.                                          |

#### Distribution

```
100 % → Always shrink when called  (ShrinkValueProbability = 1.0)
50 % → Divide by 2
50 % → Move toward zero
```

After shrinking, the value is clamped back to `[min, max]`.

---

## Echidna

### Architecture

Echidna uses a **dictionary-based generator** (`GenDict`) for fuzz and mutation,
and explicit per-type shrinkers. Pure random and dictionary selection are
governed by a single probability `pSynthA`.

### 1. Phase — Fuzz (generate from scratch)

`genAbiValueM genDict AbiUIntType` calls `genWithDict`.

#### Value Sources

| Source                  | Description                                                                                               |
| ----------------------- | --------------------------------------------------------------------------------------------------------- |
| **Pure Random**         | `getRandomUint n` (see distribution table below).                                                         |
| **AST Literals**        | `constants` map: `Map AbiType (Set AbiValue)`. Populated from AST constant declarations.                  |
| **Dynamic values**      | `dictValues` set: `Set W256`. Populated from return values of previous calls.                             |
| **Expanded edge cases** | `makeNumAbiValues i` expands every dictionary integer `i` into `±3` neighbours for all common type sizes. |

#### Distribution (Pure Random — `getRandomUint n`)

| Weight        | Range                      | Description                                                     |
| ------------- | -------------------------- | --------------------------------------------------------------- |
| 2/21 (~9.5 %) | `[0, 1023]`                | Small values (overflow/underflow hot spots).                    |
| 16/21 (~76 %) | `[0, 2^n - 5]`             | Near-full-range.                                                |
| 2/21 (~9.5 %) | `[2^n - 5, 2^n - 1]`       | Near-maximum.                                                   |
| 1/21 (~4.8 %) | Power-law (`getRandomPow`) | Exponent uniform `[20, n]`, value uniform `[2^(exp/2), 2^exp]`. |

#### Distribution (Dictionary vs Pure Random)

```
pSynthA % → Dictionary (pick from constants[AbiUIntType n])
(100 - pSynthA) % → Pure Random (getRandomUint n)
```

Default `pSynthA` is around `0.5`, so roughly a 50 / 50 split.

### 2. Phase — Mutation (mutate an existing call)

`mutateAbiCall` mutates exactly **one** randomly chosen argument.

#### Distribution

```
10 % per value → mutateAbiValue is called
  └── mutateNum x: x ± uniform[0, 2x]
90 % per value → left unchanged
```

Only one argument per call is mutated, so most of the call sequence stays
intact.

### 3. Phase — Shrink (simplify a failing input)

`shrinkAbiCall` shrinks a call by choosing a random subset of shrinkable
arguments and applying `shrinkAbiValue`.

#### Value Sources

| Source                 | Description                                                                         |
| ---------------------- | ----------------------------------------------------------------------------------- |
| **Toward zero**        | `shrinkInt x` = `uniform[0, x]`. For unsigned ints this drives directly toward `0`. |
| **Fixed alternatives** | `AbiAddress _ → {0, 0xdeadbeef}`; `AbiBool _ → False`.                              |
| **Null padding**       | `addNulls` replaces characters with `\0`.                                           |

#### Distribution

```
Per argument:
  50 % chance to be selected for shrinking (weighted by numToShrink/numShrinkable)
  └── shrinkInt: uniform [0, current_value]
```

The shrinker is probabilistic: it randomly decides how many arguments to shrink
and which ones.

---

## Foundry

### Architecture

Foundry uses **proptest** strategies. Generation is split into two paths:

1. **Fixture-based** (`fuzz_calldata`) — uses user-defined `fixture_<name>()`
   functions (10 % edge / 40 % fixtures / 50 % random).
2. **State-based** (`fuzz_calldata_from_state`) — draws from an EVM execution
   dictionary (50 % typed samples / 50 % general state values).

Mutation and shrinking are handled by proptest's built-in `ValueTree` mechanism.

### 1. Phase — Fuzz (generate from scratch)

#### Path A: `UintStrategy` (fixture mode)

#### Value Sources

| Source          | Description                                                                                          |
| --------------- | ---------------------------------------------------------------------------------------------------- |
| **Edge cases**  | `±3` around `0` and `type_max`.                                                                      |
| **Fixtures**    | User-defined `fixture_<name>()` functions returning arrays of values. Matched by **parameter name**. |
| **Pure Random** | Uniform bit-width selection: pick `b` uniform `[0, bits]`, generate two `u128`s, mask to `b` bits.   |

#### Distribution

```
10 % → Edge cases
40 % → Fixtures
50 % → Pure Random
```

#### Path B: `fuzz_param_from_state` (state mode)

#### Value Sources

| Source                   | Description                                                                                                                                                     |
| ------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Typed samples**        | `LiteralMaps.words[DynSolType::Uint(n)]` — AST literals collected per-type by the `solar` compiler. Plus typed comparison operands from sancov instrumentation. |
| **General state values** | Dynamic dictionary: storage slots, return values, event logs, and `keccak256` operands observed during execution.                                               |

#### Distribution

```
50 % → Typed samples  (per-type dictionary)
50 % → General state values  (flat dictionary)
```

When a dictionary value is selected, it is a raw `B256` word. For `uintN` where
`N < 256`, the word is reduced modulo `2^N`.

### 2. Phase — Mutation (mutate an existing call)

Foundry does **not** have an explicit per-value mutation probability like
Echidna or Medusa. Instead, proptest's strategy framework generates "similar"
values by walking the `ValueTree`.

The mutators available are:

| Mutator                   | Description                                                                                                                        |
| ------------------------- | ---------------------------------------------------------------------------------------------------------------------------------- |
| **Increment / Decrement** | `±1` (wrapping).                                                                                                                   |
| **Gaussian Noise**        | Scale the value by a random factor drawn from a Gaussian-like distribution (σ multipliers: `0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0`). |
| **Bit Flip**              | Flip one random bit.                                                                                                               |
| **Interesting Word**      | Inject AFL-style interesting bytes/words/dwords (`INTERESTING_8`, `INTERESTING_16`, `INTERESTING_32`).                             |
| **Bound**                 | Replace with a random value inside a `[min, max]` range.                                                                           |

Proptest decides which mutator to apply based on the strategy's internal logic,
not on a fixed probability table.

### 3. Phase — Shrink (simplify a failing input)

Foundry's `UintValueTree` implements binary-search shrinking:

- `simplify()` → set `hi = curr`, move `curr` toward `lo`.
- `complicate()` → set `lo = curr + 1`, move `curr` toward `hi`.

This is deterministic interval bisection. Proptest explores the tree
automatically, trying to find the smallest `curr` that still triggers the
failure.

**Note**: `fuzz_calldata_from_state` is marked `.no_shrink()` because shrinking
a raw dictionary word often produces an invalid value (e.g. an address that
violates sender filters). Fixture-based fuzzing does shrink normally.

---

## Raptor (Current)

### Architecture

Raptor's `random::uint` and `random::int` are simple three-phase functions with
no explicit shrink or mutation support.

### 1. Phase — Fuzz (generate from scratch)

#### Value Sources

| Source          | Description                                                                                    |
| --------------- | ---------------------------------------------------------------------------------------------- |
| **Literals**    | `ExtractedLiterals::numbers` — AST number literals grouped by `LiteralKind`, not by type size. |
| **Edge cases**  | `0`, `1`, `max`, `max-1`, `max-2`, `max-3`.                                                    |
| **Pure Random** | Two `u128`s ORed into a `U256`, masked to the target bit width.                                |

#### Distribution

```
40 % → Literals  (LITERAL_BIAS = 40)
30 % → Edge cases (50 % of remaining 60 %)
30 % → Pure Random
```

### 2. Phase — Mutation

Not implemented. `SharedCorpus::take()` returns either an existing item clone or
a freshly generated sequence. There is no per-argument mutation.

### 3. Phase — Shrink

Not implemented.

---

## Comparison Matrix

| Dimension               | Medusa                                          | Echidna                                                              | Foundry                                                            |
| ----------------------- | ----------------------------------------------- | -------------------------------------------------------------------- | ------------------------------------------------------------------ |
| **Fuzz — Pure Random**  | Uniform full-range bytes                        | Biased: 76 % near-full, 9.5 % small, 9.5 % near-max, 4.8 % power-law | Uniform bit-width (50 % weight in fixture mode)                    |
| **Fuzz — Literals**     | AST numbers + hard-coded `{0,1,-1,2}`           | AST constants + dynamic return values                                | AST per-type (`solar`) + dynamic EVM state                         |
| **Fuzz — Edge Cases**   | Implicit via bounds (`min`, `max`)              | `makeNumAbiValues` (±3 neighbours)                                   | Explicit `±3` around `0` and `max` (10 % weight)                   |
| **Fuzz — Fixtures**     | Not supported                                   | Not supported                                                        | User-defined `fixture_<name>()` (40 % weight)                      |
| **Fuzz — Dict Bias**    | 50 %                                            | `pSynthA` (~50 %)                                                    | 50 % (state mode)                                                  |
| **Mutation**            | 10 % chance per value; arithmetic on dictionary | 10 % chance per value; `±uniform[0,2x]`                              | Proptest mutators (bit-flip, gaussian, inc/dec, interesting words) |
| **Shrink**              | Subtract/add toward zero, or divide by 2        | `uniform[0, x]` (drive toward zero)                                  | Binary-search `ValueTree` bisection                                |
| **Type-aware literals** | No (flat dictionary)                            | No (flat dictionary)                                                 | Yes (`uint8` gets `uint8`-sized literals only)                     |

---

## Recommendations for Raptor

Based on the three-dimension model above, the highest-impact next steps are:

### Phase — Fuzz

1. **Add power-law distribution** (Echidna's `getRandomPow`). When pure random
   is selected, with some probability use a power-law instead of uniform 256-bit
   random. This makes small values far more likely.

2. **Per-type literal filtering** (Foundry's approach). Store literals under
   each `uintN` size they fit into, so a `uint8` argument never receives a
   `uint256` max literal.

### Phase — Mutation

3. **Implement per-argument mutation**. When `take()` returns a corpus item,
   apply a small mutation probability per argument (e.g. 10 %). Mutation
   methods: bit-flip, increment/decrement, arithmetic with another random
   literal (Medusa-style).

### Phase — Shrink

4. **Add `UintValueTree`-like shrinking**. When a crash is found, try to
   minimise each uint argument by binary-search bisection between `0` and the
   current value.

### Distribution

5. **Add dynamic dictionary**. During execution, collect return values, storage
   slots, and comparison operands. Feed them back into the generator (Foundry's
   state dictionary model).

## References

- Medusa `MutationalValueGenerator` —
  `external/crytic/medusa/fuzzing/valuegeneration/generator_mutational.go`
- Medusa `ShrinkingValueMutator` —
  `external/crytic/medusa/fuzzing/valuegeneration/mutator_shrinking.go`
- Medusa default configuration — `external/crytic/medusa/fuzzing/fuzzer.go`
  (`defaultCallSequenceGeneratorConfigFunc`)
- Echidna `getRandomUint` / `getRandomPow` —
  `external/crytic/echidna/lib/Echidna/ABI.hs`
- Echidna `genWithDict` / `shrinkInt` —
  `external/crytic/echidna/lib/Echidna/ABI.hs`
- Foundry `UintStrategy` —
  `external/foundry-rs/foundry/crates/evm/fuzz/src/strategies/uint.rs`
- Foundry `fuzz_param_from_state` —
  `external/foundry-rs/foundry/crates/evm/fuzz/src/strategies/param.rs`
- Foundry mutators —
  `external/foundry-rs/foundry/crates/evm/fuzz/src/strategies/mutators.rs`
- Foundry state dictionary —
  `external/foundry-rs/foundry/crates/evm/fuzz/src/strategies/state.rs`
- Raptor `random::uint` / `random::int` — `src/fuzzer/corpus/random/uint.rs` /
  `src/fuzzer/corpus/random/int.rs`
