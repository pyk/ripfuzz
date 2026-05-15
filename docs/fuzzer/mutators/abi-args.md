# ABI-Aware Argument Mutation

This document explains how Medusa and Raptor mutate Solidity function arguments
using ABI type information, and why treating calldata as raw bytes misses
interesting program states.

## Why ABI Awareness Matters

A fuzzer that sees call data as an opaque byte stream might flip arbitrary bits
in the middle of an ABI-encoded `uint8`. That can produce values like `257`,
which Solidity silently truncates to `1`. The fuzzer wastes cycles exploring
semantically equivalent inputs and never learns that `255` and `0` are
meaningfully different boundary cases.

ABI-aware mutation solves this by:

- **Preserving type boundaries** -- `uint8` mutations stay inside `[0, 255]`.
- **Targeting interesting values** -- zero, one, max, and values from the AST
  are preferred over purely random bit patterns.
- **Respecting composite structure** -- arrays, tuples, and structs are mutated
  element-wise rather than corrupted at the encoding layer.
- **Enabling shrinking** -- integer shrinking can move toward zero, and dynamic
  arrays can drop elements without breaking the ABI layout.

## Medusa's Model: Reflection and Probability-Guided Mutation

Medusa separates **generation** from **mutation** through two interfaces:

```go
type ValueGenerator interface {
    GenerateAddress() common.Address
    GenerateInteger(signed bool, bitLength int) *big.Int
    GenerateBool() bool
    GenerateString() string
    GenerateBytes() []byte
    GenerateFixedBytes(length int) []byte
    GenerateArrayOfLength() int
}

type ValueMutator interface {
    MutateAddress(addr common.Address) common.Address
    MutateInteger(i *big.Int, signed bool, bitLength int) *big.Int
    MutateBool(bl bool) bool
    MutateString(s string) string
    MutateBytes(b []byte) []byte
    MutateFixedBytes(b []byte) []byte
    MutateArray(value []any, fixedLength bool) []any
}
```

### `MutateAbiValue`: The Dispatch Hub

Every argument mutation flows through `MutateAbiValue` in
`fuzzing/valuegeneration/abi_values.go`. It receives:

- a `ValueGenerator` (for creating new values when an array slot is nil),
- a `ValueMutator` (for transforming existing values),
- an `abi.Type` (to know the Solidity type at runtime),
- and the current Go value (decoded via reflection).

```go
func MutateAbiValue(
    generator ValueGenerator,
    mutator ValueMutator,
    inputType *abi.Type,
    value any,
) (any, error)
```

The function switches on `inputType.T` (address, uint, int, bool, string, bytes,
fixed bytes, array, slice, tuple) and performs three things:

1. **Type assertion** -- cast the `any` value to the concrete Go type that
   g-ethereum's ABI decoder produced (e.g., `*big.Int` for `uint256`,
   `common.Address` for `address`).
2. **Dispatch to the mutator** -- call the matching `Mutate*` method.
3. **Recursion** -- for arrays, slices, and tuples, recursively mutate each
   element, generating fresh values for any nil slots.

After mutation, Medusa re-encodes the whole argument list back into calldata via
`element.Call.WithDataAbiValues(...)`.

### `MutationalValueGenerator`: The Workhorse

The default fuzzing mutator is `MutationalValueGenerator` in
`fuzzing/valuegeneration/generator_mutational.go`. It combines random generation
with seeded mutation:

- **ValueSet** -- a corpus of literals extracted from the AST (constants, event
  signatures, hard-coded addresses) plus bootstrap values `0`, `1`, `-1`, `2`.
  The mutator draws from this set instead of always using pure randomness.
- **Bias probabilities** -- every type has a config-driven probability of being
  mutated at all (`MutateIntegerProbability`, `MutateBytesProbability`, etc.)
  and a separate bias for replacing the value with a completely new random one
  rather than mutating the existing one (`MutateIntegerGenerateNewBias`).
- **Multiple rounds** -- `MinMutationRounds` and `MaxMutationRounds` define how
  many successive mutation functions are applied to a single starting value.

#### Integer Mutation

`mutateIntegerInternal` picks a base value from the `ValueSet` or the input,
then applies a random number of operations from this list:

```go
var integerMutationMethods = []func(*MutationalValueGenerator, *big.Int, ...*big.Int) *big.Int{
    func(g, x, inputs) { return x + random_input },      // add
    func(g, x, inputs) { return x - random_input },      // subtract
    func(g, x, inputs) { return x * random_input },      // multiply
    func(g, x, inputs) { return x / random_input },      // divide
    func(g, x, inputs) { return x % random_input },      // modulo
}
```

After each operation the result is clamped back into the valid bit range with
`ConstrainIntegerToBounds` so that underflows and overflows are corrected.

#### Bytes and String Mutation

Bytes and strings have their own method tables that perform structural edits:
replace a random byte, flip a random bit, insert a byte, or remove a byte.
Strings operate on runes and stay within printable ASCII bounds.

#### Array Mutation

`MutateArray` currently has a TODO for structural mutations (swap, insert,
delete), but the scaffolding is already in place: it checks
`MutateArrayStructureProbability`, determines a mutation count, and then
recursively mutates each element.

### `ShrinkingValueMutator`: Reducing Inputs

During shrinking Medusa swaps the normal mutator for `ShrinkingValueMutator` in
`fuzzing/valuegeneration/mutator_shrinking.go`. It has a single probability
parameter, `ShrinkValueProbability`, and only applies "shrink" operations:

- **Integers** -- subtract a random value-set entry if positive, add if
  negative, or divide by two.
- **Bytes** -- zero out a random byte, or remove a random byte.
- **Strings** -- replace a random rune with NULL, or remove a random rune.
- **Addresses, bools, fixed bytes, arrays** -- untouched (no shrink strategy
  defined yet).

This means Medusa's shrinking is **type-aware** as well: it knows that removing
a byte from a `bytes` argument shortens the payload, and that moving an integer
toward zero is likely to simplify the input.

### Sequence Generator Integration

Inside `fuzzing/fuzzer_worker_sequence_generator.go`, the call-sequence
generator has a dedicated mutation phase:

```go
func prefetchModifyCallFuncMutate(
    sequenceGenerator *CallSequenceGenerator,
    element *calls.CallSequenceElement,
) error {
    abiValuesMsgData := element.Call.DataAbiValues
    for i := 0; i < len(abiValuesMsgData.InputValues); i++ {
        mutatedInput, err := valuegeneration.MutateAbiValue(
            sequenceGenerator.config.ValueGenerator,
            sequenceGenerator.config.ValueMutator,
            &abiValuesMsgData.Method.Inputs[i].Type,
            abiValuesMsgData.InputValues[i],
        )
        if err != nil {
            return fmt.Errorf("error when mutating call sequence input argument: %v", err)
        }
        abiValuesMsgData.InputValues[i] = mutatedInput
    }
    element.Call.WithDataAbiValues(abiValuesMsgData)
    return nil
}
```

Because Medusa keeps arguments as decoded Go objects (`DataAbiValues`), it can
mutate them in place and then re-encode. The raw calldata is never touched
directly.

## Raptor's Implementation

Raptor's ABI argument mutator lives in `src/fuzzer/mutators/abi/arg.rs`. It is a
LibAFL `Mutator<CallSequenceInput, S>` that works by decoding the raw ABI buffer
into `DynSolValue`s, mutating recursively, and re-encoding.

### `SequenceArgMutator`

```rust
#[derive(Debug)]
pub struct SequenceArgMutator {
    abi: JsonAbi,
}
```

The mutator holds the contract's `JsonAbi` (from `alloy-json-abi`). When asked
to mutate a `CallSequenceInput` it:

1. **Picks a random call** from the sequence.
2. **Looks up the function** by its 4-byte selector.
3. **Parses input types** into `DynSolType` via `selector_type().parse()`.
4. **Decodes** the raw `call.args` buffer with `abi_decode_params`.
5. **Recursively mutates** each `DynSolValue`.
6. **Re-encodes** with `abi_encode_params` and stores the new buffer back into
   the call.

### `mutate_value`: Per-Type Rules

Raptor's recursive mutator matches on `DynSolValue` variants directly:

| Type                   | Mutation Rule                                                                                         |
| ---------------------- | ----------------------------------------------------------------------------------------------------- |
| `Uint(v, sz)`          | Add or subtract a delta in `[-500, +499]`. If `sz < 256`, mask with `(1 << sz) - 1` to stay in range. |
| `Int(v, _sz)`          | Add or subtract a delta in `[-500, +499]` using `I256` wrapping arithmetic.                           |
| `Bool(b)`              | Flip the boolean: `!b`.                                                                               |
| `Address(a)`           | Randomize the last 20 bytes (bytes 12..32), keeping the 12-byte padding intact.                       |
| `Function(f)`          | Pick a random byte in the 24-byte function pointer and overwrite it.                                  |
| `Bytes(b)`             | If non-empty, overwrite one random byte.                                                              |
| `String(s)`            | Overwrite one random byte in the UTF-8 payload. Uses `from_utf8_lossy` so invalid UTF-8 is harmless.  |
| `FixedBytes(word, sz)` | Overwrite one random byte among the first `sz` bytes.                                                 |
| `Array` / `FixedArray` | With probability `1/4`, swap two random elements; then recursively mutate every element.              |
| `Tuple`                | Recursively mutate every field.                                                                       |

### Key Differences from Medusa

- **Single-pass mutation** -- Raptor applies exactly one delta or one byte flip
  per invocation, rather than Medusa's configurable multi-round loop.
- **No ValueSet** -- Raptor does not seed mutations with AST literals or
  interesting constants. It relies on the fuzzing engine's corpus and coverage
  guidance to discover useful values.
- **No explicit shrinking mutator** -- LibAFL's generic minimization (e.g.
  `MinimizerMutator`) operates on the raw byte representation of the whole
  sequence. Raptor does not have a dedicated ABI-aware shrinking pass like
  Medusa's `ShrinkingValueMutator`.
- **Type masking** -- Raptor explicitly masks narrow integers (`uint8`,
  `uint16`, etc.) after mutation, guaranteeing that the re-encoded value is
  always valid for its Solidity type. Medusa achieves the same with
  `ConstrainIntegerToBounds`.

### Safety Guarantees

Because Raptor decodes the full ABI buffer into typed values and re-encodes
after mutation, the offset/length metadata of dynamic types (`bytes`, `string`,
arrays) is automatically preserved. A test in `arg.rs` verifies this:

```rust
// Proper ABI encoding for setData(hex"abcd"):
// word 0: offset = 32
// word 1: length = 2
// word 2: data = 0xabcd padded
```

After mutation, the offset and length words remain unchanged; only the payload
bytes are altered. This is the same safety property Medusa gets by working with
decoded Go objects.

### Integration with LibAFL

`SequenceArgMutator` implements LibAFL's `Mutator` trait:

```rust
impl<S: HasRand> Mutator<CallSequenceInput, S> for SequenceArgMutator {
    fn mutate(
        &mut self,
        state: &mut S,
        input: &mut CallSequenceInput,
    ) -> Result<MutationResult, libafl::Error> {
        // ... pick random call, mutate args, return Mutated or Skipped
    }
}
```

It is typically composed with other mutators (block-delay mutator, sender
mutator, sequence-order mutator) inside a `StdMOptMutator` or
`StdScheduledMutator` so that the fuzzing engine can choose which mutation
strategy to apply based on coverage feedback.

## Comparison Summary

| Feature            | Medusa                                          | Raptor                                    |
| ------------------ | ----------------------------------------------- | ----------------------------------------- |
| Type system        | `abi.Type` + Go reflection                      | `DynSolType` / `DynSolValue`              |
| Mutation target    | Decoded Go values                               | Decoded `DynSolValue`s                    |
| Re-encoding        | `WithDataAbiValues`                             | `abi_encode_params`                       |
| Seeded values      | `ValueSet` (AST literals, bootstrap constants)  | None (relies on corpus)                   |
| Multi-round        | Yes (`MinMutationRounds` / `MaxMutationRounds`) | No (single pass)                          |
| Probability biases | Per-type mutation and replacement probabilities | Always mutates when invoked               |
| Integer operations | Add, sub, mul, div, mod                         | Add or sub fixed delta `[-500, 499]`      |
| Shrinking          | Dedicated `ShrinkingValueMutator`               | Generic LibAFL minimization               |
| Array structure    | Swap/insert/delete TODO                         | Element swap + recursive element mutation |
| String mutation    | ASCII-biased rune edits                         | Raw byte flip                             |

## What Raptor Is Missing

1. **ValueSet seeding** -- Raptor does not extract Solidity constants, event
   selectors, or other contract-specific literals to bias mutation. Adding a
   `ValueSet` equivalent could improve time-to-bug for contracts with magic
   numbers.
2. **Multi-round mutation** -- Raptor always applies a single small delta.
   Configurable rounds would allow deeper local exploration around a promising
   value.
3. **Dedicated shrinking mutator** -- LibAFL's generic minimizers are effective
   for raw sequences, but an ABI-aware shrinker could, for example, replace a
   `uint256` with `0` or truncate a dynamic array, producing more readable
   failing cases faster.
4. **Probability configuration** -- Raptor currently mutates every type with
   fixed deterministic rules. Exposing probabilities (e.g. "flip a bool only 30%
   of the time") would let users tune mutation aggressiveness.
