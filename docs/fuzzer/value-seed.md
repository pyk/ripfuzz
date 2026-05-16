# Value Seeding for the ABI-Aware Mutator

This document describes how Medusa seeds the value set that powers its ABI-aware
mutational generator. Understanding this mechanism is required to implement an
equivalent value-seeding pipeline in Raptor.

## Overview

Medusa does not generate function arguments from pure entropy. Instead, it
maintains a `ValueSet`---a collection of semantically interesting values
extracted from the target contract and from runtime execution. The mutational
generator uses this set as a _seed pool_: it picks base values from the pool and
applies type-aware mutations, or returns them verbatim. This dramatically
increases the probability of generating inputs that exercise boundary
conditions, literal comparisons, and address-dependent logic.

The value set is split into four independent collections:

| Collection  | Go type                  | Purpose                                                    |
| ----------- | ------------------------ | ---------------------------------------------------------- |
| `addresses` | `map[common.Address]any` | EOAs, deployed contracts, and literal addresses            |
| `integers`  | `map[string]*big.Int`    | Numeric constants and runtime return values                |
| `strings`   | `map[string]any`         | String literals and runtime return values                  |
| `bytes`     | `map[string][]byte`      | Raw byte sequences (hashed by Keccak256 for deduplication) |

Maps are used for deduplication; each collection exposes a slice accessor
(`Addresses()`, `Integers()`, `Strings()`, `Bytes()`).

## Seed Sources

Seeds enter the `ValueSet` through two phases: **static seeding** (once, at
fuzzer startup) and **runtime seeding** (continuously, during fuzzing).

### Static Seeds

Static seeds are collected before the first worker starts.

#### 1. Hardcoded Constants

Both the mutational generator and the shrinking mutator inject a small set of
"universal" integers into every fresh `ValueSet`:

- **MutationalValueGenerator**: `0`, `1`, `-1`, `2`
- **ShrinkingValueMutator**: `0`, `1`, `2`

These values are required for basic mutation operations (e.g. adding or
subtracting `1` from an existing value, or shrinking toward zero).

#### 2. Deployer and Sender Addresses

When the `Fuzzer` is constructed, the following addresses are added to the
**base** `ValueSet`:

- The deployer address (`fuzzer.deployer`).
- Every sender address in `fuzzer.senders`.

These ensure that fuzzed calls can target the fuzzer-controlled accounts.

#### 3. Slither Constants (`SeedFromSlither`)

If Slither is enabled and succeeds, Medusa parses `slither.Constants` and seeds
the base `ValueSet`:

| Slither `constant.Type` | Action on `ValueSet`                                     |
| ----------------------- | -------------------------------------------------------- |
| `uint*`, `int*`         | `AddInteger(b)`, `AddInteger(-b)`, `AddBytes(b.Bytes())` |
| `bool`                  | `AddInteger(0)` if `False`, else `AddInteger(1)`         |
| `string`                | `AddString(value)`, `AddBytes([]byte(value))`            |
| `address`               | `AddAddress(BigToAddress(b))`, `AddBytes([]byte(value))` |

This captures hardcoded numeric thresholds, magic strings, and address literals
that the contract author embedded in source code.

#### 4. AST Literals (`SeedFromAst`)

If Slither fails or is unavailable, Medusa falls back to walking the Solidity
AST. For every `Literal` node:

- **Number literals** (`kind == "number"`)
    - Hex prefix (`0x...`): parsed as `big.Int` (base 16). The value, its
      negation, and its `BigToAddress` conversion are added.
    - Decimal: parsed with `shopspring/decimal` to preserve precision. If a
      `subdenomination` is present (wei, gwei, szabo, finney, ether, seconds,
      minutes, hours, days, weeks, years), the literal is multiplied accordingly
      before conversion to `big.Int`. The absolute value, its negation, and its
      `BigToAddress` conversion are added.
- **String literals** (`kind == "string"`): `AddString(literalValue)`.

The AST walker recurses over every map and slice in the AST structure, so
literals buried inside nested expressions or arrays are still extracted.

#### 5. Genesis Contract Mappings

During worker chain setup, any address specified in the genesis configuration
under `GenesisContractMappings` is added to the **worker** `ValueSet`. This lets
the fuzzer call pre-deployed system contracts or cheat-code addresses.

### Runtime Seeds

Runtime seeds are collected by each `FuzzerWorker` while it executes call
sequences. They are **not** shared globally; each worker maintains its own
cloned `ValueSet`.

#### 1. Deployed Contract Addresses

Whenever a contract is deployed on the worker's chain:

- `onChainContractDeploymentAddedEvent` adds the deployment address to the
  worker's `ValueSet`.
- If the contract is later removed (e.g. via `selfdestruct` or chain revert),
  `onChainContractDeploymentRemovedEvent` deletes it.

This guarantees that `GenerateAddress()` can return addresses of contracts that
actually exist in the current state.

#### 2. Function Return Values

After each call in a sequence is executed, the worker attempts to decode the
return values:

```go
decodedReturnValues, err := latestCallSequenceElement.DecodedReturnValues()
if decodedReturnValues != nil && err == nil {
    fw.valueSet.Add(decodedReturnValues)
}
```

`ValueSet.Add` accepts a slice of `any` and type-switches over primitives:

- `uint8/16/32/64`, `int8/16/32/64`, `*big.Int` -> `AddInteger`
- `common.Address` -> `AddAddress`
- `bool` -> `AddInteger(0)` or `AddInteger(1)`
- `string` -> `AddString`
- `[]byte` -> `AddBytes`
- Fixed-size byte arrays (reflection) -> converted to slice, then `AddBytes`

Return values from view functions, state-changing functions, and cheat-code
calls therefore feed back into the mutation pool.

#### 3. ValueSet Rollback

Because runtime seeds are local to a worker and to a single call sequence,
Medusa rolls back after each sequence test:

```go
originalValueSet := fw.valueSet.Clone()
defer func() {
    fw.valueSet = originalValueSet
}()
```

This prevents "pollution" across sequences: a return value from one sequence
will not permanently dominate the value set of future sequences.

## How Seeds Are Consumed by the Mutational Generator

The `MutationalValueGenerator` implements both `ValueGenerator` (create new
values) and `ValueMutator` (mutate existing ones). It delegates to a
`RandomValueGenerator` when it decides to ignore the seed pool.

### Generation vs. Mutation Decision Flow

For every type, the generator first flips a biased coin:

- **Generation**: `randomProvider.Float32() < config.GenerateRandom*Bias` -> use
  pure random generator.
- **Mutation of existing value**:
  `randomProvider.Float32() < config.Mutate*Probability` -> either mutate the
  existing value or replace it with a freshly generated one
  (`*GenerateNewBias`).
- **Otherwise**: return the original value unchanged.

### Integer Mutation

When mutating an integer:

1. Gather inputs: all integers in the `ValueSet`, plus the type's `min` and
   `max` bounds (only `max` for unsigned, because `0` is already in the set).
2. Pick a random starting value from the input list, or use the provided
   existing value.
3. Constrain it to `[min, max]`.
4. Perform `mutationCount` rounds (0 to 1 by default) of:
    - `add(x, random_input)`
    - `sub(x, random_input)`
    - `mul(x, random_input)`
    - `div(x, random_input)` (guard against divide-by-zero)
    - `mod(x, random_input)` (guard against divide-by-zero)
5. Re-constrain to `[min, max]` after each round.

If no existing input is provided (i.e. `GenerateInteger` is called), the
generator picks a random base value from the `ValueSet` and mutates it.

### Bytes Mutation

For dynamic or fixed-size bytes:

1. Gather all byte slices from `ValueSet.Bytes()`.
2. Pick a random base slice, or use the existing one.
3. Perform `mutationCount` rounds of:
    - **Replace**: overwrite a random index with a random byte.
    - **Bit flip**: flip a random bit in a random byte.
    - **Insert**: add a random byte at a random position.
    - **Remove**: delete a random byte.

If the requested output is fixed-size, the result is zero-padded or truncated to
the required length.

### String Mutation

For strings:

1. Gather all strings from `ValueSet.Strings()`.
2. Pick a random base string, or use the existing one.
3. Perform `mutationCount` rounds of:
    - **Replace character**: overwrite a random rune with a printable ASCII rune
      in `[32, 126]`.
    - **Bit flip**: flip a random bit in a random rune.
    - **Insert character**: insert a printable ASCII rune at a random position.
    - **Remove character**: delete a random rune.

### Address Generation

`GenerateAddress` simply returns a random address from `ValueSet.Addresses()`.
If the set is empty, it falls back to `RandomValueGenerator.GenerateAddress()`
(20 random bytes). `MutateAddress` replaces the address with a random one with
probability `MutateAddressProbability`.

### Array Mutation

`MutateArray` currently has a placeholder for structural mutations (swap,
insert, delete) but does not apply them. It returns the input slice unchanged,
except for the probability gate.

### Bool Mutation

`MutateBool` returns a random boolean with probability `MutateBoolProbability`.

## Default Configuration

The default `MutationalValueGeneratorConfig` used by
`defaultCallSequenceGeneratorConfigFunc` is:

```go
MinMutationRounds:               0
MaxMutationRounds:               1
GenerateRandomAddressBias:       0.05
GenerateRandomIntegerBias:       0.50
GenerateRandomStringBias:        0.05
GenerateRandomBytesBias:         0.05
MutateAddressProbability:        0.10
MutateArrayStructureProbability: 0.10
MutateBoolProbability:           0.10
MutateBytesProbability:          0.10
MutateBytesGenerateNewBias:      0.45
MutateFixedBytesProbability:     0.10
MutateStringProbability:         0.10
MutateStringGenerateNewBias:     0.70
MutateIntegerProbability:        0.10
MutateIntegerGenerateNewBias:    0.50
```

And the underlying `RandomValueGeneratorConfig`:

```go
GenerateRandomArrayMinSize:  0
GenerateRandomArrayMaxSize:  100
GenerateRandomBytesMinSize:  0
GenerateRandomBytesMaxSize:  100
GenerateRandomStringMinSize: 0
GenerateRandomStringMaxSize: 100
```

## Shrinking Value Mutator Seeding

The `ShrinkingValueMutator` reuses the same `ValueSet` but has a different
mutation philosophy: it only _reduces_ values to make test cases smaller. It
also injects `0`, `1`, `2` into its copy of the set.

- **Integer shrinking**:
    - If positive: `sub(x, random_input)` (move toward zero).
    - If negative: `add(x, random_input)` (move toward zero).
    - `div(x, 2)` (halve).
- **Bytes shrinking**:
    - Replace a random byte with `0x00`.
    - Remove a random byte.
- **String shrinking**:
    - Replace a random rune with `NULL` (`\0`).
    - Remove a random rune.

Addresses, booleans, fixed bytes, and arrays are left untouched by the shrinking
mutator.

## Implementation Checklist for Raptor

To replicate Medusa's seeding in Raptor:

1. **ValueSet container**
    - Deduplicating stores for addresses, integers, strings, and byte arrays.
    - Clone capability (deep copy) for per-worker, per-sequence isolation.

2. **Static seeding pipeline**
    - Hardcode `0`, `1`, `-1`, `2` into every fresh `ValueSet`.
    - Add deployer and all sender EOAs.
    - Integrate a Slither parser: extract `constants` and map types to the four
      collections.
    - Implement an AST walker that recurses over JSON AST nodes, detects
      `Literal` nodes, parses hex/dec numbers (with denominations), and extracts
      strings.
    - Add genesis-mapped contract addresses.

3. **Runtime seeding pipeline**
    - Hook into contract-deployment events: `AddAddress` on deployment,
      `RemoveAddress` on destruction.
    - Hook into call execution: decode return values and feed them through
      `ValueSet.Add`.
    - Clone the `ValueSet` before each sequence test and restore it afterward.

4. **Mutational generator**
    - Implement bias-based branching between random generation, mutation, and
      no-op.
    - Integer mutation: arithmetic ops against random inputs from the value set,
      with bit-length boundary clamping.
    - Bytes mutation: replace, bit-flip, insert, remove.
    - String mutation: character-level replace, bit-flip, insert, remove over
      runes.
    - Address generation: uniform random selection from `ValueSet.Addresses()`,
      fallback to random bytes.

5. **Shrinking mutator**
    - Reuse the worker's `ValueSet`.
    - Implement only shrink-oriented mutations (zeroing, halving, deletion).

6. **ABI bridge**
    - `GenerateAbiValue` must dispatch to the generator by `abi.Type`.
    - `MutateAbiValue` must recursively walk the ABI type, mutate primitives via
      the mutator, and regenerate `nil` array elements.
