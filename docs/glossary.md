# Glossary

Consistent vocabulary for raptor users and contributors.

## Core Terms

### Fuzzing Campaign

A single invocation of `raptor fuzz`. A campaign initializes the **target
contract**, builds seed inputs, and orchestrates one or more **fuzzers** that
generate sequences of **function calls**, execute them against a cloned contract
state, and check that all **properties** still hold. Also called a "fuzz run" or
"test run".

### Target Contract

The Solidity file you pass to `raptor fuzz` (e.g. `./test/CounterTarget.sol`).
It is the contract raptor compiles, deploys, and exercises. Also called a
**handler contract** in some tooling communities.

### Invariant Function

A Solidity function that encodes an invariant. By default it must:

- start with the prefix `invariant_`
- take no arguments
- be `pure` or `view`

Raptor appends every invariant to the end of each function call sequence and
executes it in the same EVM loop. If an invariant reverts with a Solidity
`assert` failure (`Panic(0x01)`), the fuzzer records a crash. The return value,
if any, is ignored. Synonyms: **invariant**, **property test**.

### Target Function

Any external or public function in the target contract that is _not_ a setup or
invariant function. Raptor calls these with randomly-generated arguments to
mutate contract state. A single fuzz input is a **sequence of function calls**.
Synonyms: **function call**, **handler function** (Foundry).

### Setup Function

A function that establishes the initial state cloned for every fuzz input. The
contract **constructor** always runs once at deployment. If a function named
`setup()` exists, raptor calls it once after deployment.

### Fuzzer

A single parallel fuzzing instance that executes function call sequences against
a cloned contract state and reports new coverage or crashes to the campaign
manager. By default raptor spawns one fuzzer per available CPU core.

### Campaign Result

The aggregated output of a fuzzing campaign, including the total number of
iterations executed across all fuzzers and any crashes (assert panics)
discovered.

### Crash

A failure recorded when any call (target function or invariant) reverts with a
Solidity `assert` panic (`Panic(0x01)`). The fuzzer treats a crash as a bug and
adds it to the set of objectives. Reverts caused by `require` or other reasons
do not produce a crash. Synonyms: **objective**, **bug**.

## Correspondence with Other Fuzzers

| Raptor          | Foundry (invariant) | Medusa        | Echidna       |
| --------------- | ------------------- | ------------- | ------------- |
| Target          | Handler             | Target        | Target        |
| `invariant_`    | `invariant_`        | `property_`   | `echidna_`    |
| Target Function | Handler function    | Function call | Function call |
| Campaign        | Test run            | Fuzzing run   | Test run      |
