# Glossary

Consistent vocabulary for raptor users and contributors.

## Core Terms

### Fuzzing Campaign

A single invocation of `raptor fuzz`. A campaign initializes the **target
contract**, builds seed inputs, and orchestrates one or more **workers** that
generate sequences of **function calls**, execute them against a cloned contract
state, and check that all **properties** still hold. Also called a "fuzz run" or
"test run".

### Target Contract

The Solidity file you pass to `raptor fuzz` (e.g. `./test/CounterTarget.sol`).
It is the contract raptor compiles, deploys, and exercises. Also called a
**handler contract** in some tooling communities.

### Property Function

A Solidity function that encodes an invariant. By default it must:

- start with the prefix `property_`
- take no arguments
- return `bool`

Raptor calls every property after each function call sequence. If any returns
`false`, the fuzzer records a bug. Synonyms: **invariant**, **property test**.

### Function Call (Fuzzed Function)

Any external or public function in the target contract that is _not_ a setup or
property function. Raptor calls these with randomly-generated arguments to
mutate contract state. A single fuzz input is a **sequence of function calls**.

### Setup Function

A function that establishes the initial state cloned for every fuzz input. The
contract **constructor** always runs once at deployment. If a function named
`setUp()` exists, raptor calls it once after deployment.

### Worker

A single parallel fuzzing process that executes function call sequences against
a cloned contract state and reports new coverage or crashes to the campaign
manager. By default raptor spawns one worker per available CPU core.

### Campaign Result

The aggregated output of a fuzzing campaign, including the total number of
iterations executed across all workers and any crashes (property failures)
discovered.

### Crash

A failure recorded when a property function returns `false` or reverts during
its check. The fuzzer treats a crash as a bug and adds it to the set of
objectives. Synonyms: **objective**, **bug**.

## Correspondence with Other Fuzzers

| Raptor        | Foundry (invariant) | Medusa        | Echidna       |
| ------------- | ------------------- | ------------- | ------------- |
| Target        | Handler             | Target        | Target        |
| `property_`   | `invariant_`        | `property_`   | `echidna_`    |
| Function Call | Handler function    | Function call | Function call |
| Campaign      | Test run            | Fuzzing run   | Test run      |
