# Target Contract Conventions

This document defines how to write a Solidity contract that `raptor fuzz` can
fuzz.

## Overview

A raptor target contract is a normal Solidity contract with **three kinds of
functions**:

1. **Setup Functions**: initialize state before fuzzing begins
2. **Fuzzed Functions**: function calls the fuzzer can make to mutate state
3. **Invariant Functions**: invariants the fuzzer checks after every call
   sequence

## Example

```solidity
// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

contract CounterTarget {
    uint256 public count;

    // -------------------------------------------------
    // 1. SETUP
    // -------------------------------------------------
    // The constructor (or a setUp() call) establishes
    // the initial world state. Raptor deploys the
    // contract once and then clones that state for
    // every fuzz input.
    function setUp() external {
        count = 0;
    }

    // -------------------------------------------------
    // 2. FUZZED FUNCTIONS (FUNCTION CALLS)
    // -------------------------------------------------
    // Any external/public function that does NOT match
    // an invariant prefix is a function call. Raptor will call
    // these with type-appropriate random inputs.

    function increment() external {
        count += 1;
    }

    function decrement() external {
        require(count > 0, "underflow");
        count -= 1;
    }

    function add(uint256 x) external {
        count += x;
    }

    // -------------------------------------------------
    // 3. INVARIANT FUNCTIONS
    // -------------------------------------------------
    // Functions with the `invariant_` prefix, no arguments,
    // and declared `pure` or `view`. Raptor appends these
    // to the end of every function call sequence. If any
    // reverts with an `assert` panic, raptor reports a bug.

    function invariant_count_never_overflows() external view {
        assert(count >= count + 1); // sanity check
    }

    function invariant_count_stays_small() external view {
        assert(count < 1000);
    }
}
```

## 1. Setup Functions

Setup establishes the **base state** that every fuzz input starts from.

### What counts as setup

- The contract **constructor** — always runs once at deployment
- A function named **`setUp()`** — raptor calls this automatically after
  deployment if it exists

### Rules

- Setup runs **exactly once** per fuzz campaign
- The resulting state is **cloned** for every fuzz input
- Setup functions should not be called by the fuzzer as regular function calls

### Example with setUp()

```solidity
contract LendingTarget {
    Token public token;
    LendingPool public pool;

    function setUp() external {
        token = new Token();
        pool = new LendingPool(address(token));
        token.mint(address(pool), 1_000_000 ether);
    }
}
```

## 2. Fuzzed Functions (Function Calls)

These are the functions raptor calls with random inputs to explore state space.

### Discovery rules

A function is treated as a function call if **all** of these are true:

- It is `external` or `public`
- It is **not** a setup function (`setUp`)
- It does **not** match an invariant prefix (see below)

### Input generation

Raptor generates ABI-encoded calldata for each function call:

- The 4-byte selector is fixed (from the ABI)
- Arguments are generated according to their Solidity type:
    - `uint256` → random 32-byte value
    - `address` → random 20-byte value
    - `bool` → random 0 or 1
    - `bytes` → random length-prefixed blob
    - Structs/tuples → recursively generated

### Multiple function calls per input

A single fuzz input is a **sequence of function calls** (default: up to 32
calls). This lets raptor explore stateful interactions.

```solidity
// A single fuzz input might do:
// 1. deposit(100)
// 2. borrow(50)
// 3. deposit(200)
// 4. borrow(999)   // <- maybe this triggers an invariant violation
```

## 3. Invariant Functions

Invariant functions are **state checks** that must always hold.

### Naming convention

By default, invariant functions must match the prefix:

```
invariant_
```

Example: `invariant_balance_positive`, `invariant_no_reentrancy`

### Signature requirements

A valid invariant function **must** have this signature shape:

```solidity
function invariant_<name>() external view
```

Requirements:

- Name starts with `invariant_` (configurable)
- Takes **no arguments**
- Is `view` or `pure` (read-only)
- Return type is optional and ignored

### Semantics

- Raptor appends **all** invariant calls to the end of every function call
  sequence and executes them in the same EVM loop
- If **any** invariant reverts with a Solidity `assert` panic (`Panic(0x01)`),
  the fuzzer records a **crash** (objective)
- Reverts caused by `require` or other reasons set `all_ok = false` but do
  **not** produce a crash
- Invariants are **not** called as fuzzed function calls (they are appended,
  not randomly generated)

### Example invariants

```solidity
function invariant_solvency() external view {
    assert(token.balanceOf(address(pool)) >= pool.totalDeposits());
}

function invariant_user_cant_borrow_more_than_deposited() external view {
    assert(pool.totalBorrows() <= pool.totalDeposits());
}
```

## Fuzzing Lifecycle

For each fuzz input, raptor performs this exact sequence:

```
1. CLONE the post-setup state
2. BUILD the call sequence:
   - fuzzed calls (randomly generated or mutated)
   - invariant calls (appended automatically)
3. EXECUTE every call in a single loop
4. After each call:
   - Succeeded → continue
   - Reverted with assert panic → CRASH (BUG!)
   - Reverted for any other reason → all_ok = false, break
5. RECORD result:
   - New coverage → add to corpus
   - Assert panic detected → add to objectives (BUG!)
   - Normal revert → not a bug
6. RESET state (discard clone, go back to base)
```

## Comparison with Other Fuzzers

| Feature          | Raptor                  | Foundry (invariant) | Medusa                 | Echidna                 |
| ---------------- | ----------------------- | ------------------- | ---------------------- | ----------------------- |
| Setup            | `constructor`/`setUp()` | `setUp()`           | Deployment + `setUp()` | `constructor`/`setUp()` |
| Function Calls   | All external/public     | Handlers            | All external/public    | All external/public     |
| Invariant prefix | `invariant_`            | `invariant_`        | `property_`            | `echidna_`              |
| Invariant args   | None                    | None                | None                   | None                    |
| Invariant returns| Ignored                 | `bool`              | `bool`                 | `bool`                  |
| Bug on           | `assert` panic          | Invariant `false`   | Property `false`       | Property `false`        |
| Bug on revert    | `assert` only           | No                  | No                     | No                      |

## Configuration

Invariant prefixes and other behavior can be configured in `raptor.toml`:

```toml
[fuzzing.testing.invariant]
enabled = true
prefixes = ["invariant_"]
```
