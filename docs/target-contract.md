# Target Contract Conventions

This document defines how to write a Solidity contract that `raptor fuzz` can
fuzz.

## Overview

A raptor target contract is a normal Solidity contract with **three kinds of
functions**:

1. **Setup Functions**: initialize state before fuzzing begins
2. **Fuzzed Functions**: actions the fuzzer can call to mutate state
3. **Property Functions**: invariants the fuzzer checks after every call
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
    // 2. FUZZED FUNCTIONS (actions)
    // -------------------------------------------------
    // Any external/public function that does NOT match
    // a property prefix is an action. Raptor will call
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
    // 3. PROPERTY FUNCTIONS (invariants)
    // -------------------------------------------------
    // Functions with the `property_` prefix, no arguments,
    // and a `bool` return value. Raptor calls these
    // after every action sequence. If any returns `false`,
    // raptor reports a bug.

    function property_count_never_overflows() external view returns (bool) {
        return count >= count + 1; // sanity check
    }

    function property_count_stays_small() external view returns (bool) {
        return count < 1000;
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
- Setup functions should not be called by the fuzzer as regular actions

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

## 2. Fuzzed Functions (Actions)

Actions are the functions raptor calls with random inputs to explore state
space.

### Discovery rules

A function is treated as an action if **all** of these are true:

- It is `external` or `public`
- It is **not** a setup function (`setUp`)
- It does **not** match a property prefix (see below)
- It does **not** match an assertion prefix (reserved for assertion tests)

### Input generation

Raptor generates ABI-encoded calldata for each action:

- The 4-byte selector is fixed (from the ABI)
- Arguments are generated according to their Solidity type:
    - `uint256` → random 32-byte value
    - `address` → random 20-byte value
    - `bool` → random 0 or 1
    - `bytes` → random length-prefixed blob
    - Structs/tuples → recursively generated

### Multiple actions per input

A single fuzz input is a **sequence of actions** (default: up to 32 calls). This
lets raptor explore stateful interactions.

```solidity
// A single fuzz input might do:
// 1. deposit(100)
// 2. borrow(50)
// 3. deposit(200)
// 4. borrow(999)   // <- maybe this triggers an invariant violation
```

## 3. Property Functions (Invariants)

Property functions are **boolean checks** that must always hold.

### Naming convention

By default, property functions must match the prefix:

```
property_
```

Example: `property_balance_positive`, `property_no_reentrancy`

### Signature requirements

A valid property function **must** have exactly this signature:

```solidity
function property_<name>() external view returns (bool)
```

Requirements:

- Name starts with `property_` (configurable)
- Takes **no arguments**
- Returns exactly one `bool`
- Is `view` or `pure` (read-only)

### Semantics

- Raptor calls **all** property functions after every action sequence
- If **any** property returns `false`, the fuzzer records a **crash**
  (objective)
- If a property itself reverts, that is also treated as a failure
- Properties are **not** called as actions (they are checked, not fuzzed)

### Example properties

```solidity
function property_solvency() external view returns (bool) {
    return token.balanceOf(address(pool)) >= pool.totalDeposits();
}

function property_user_cant_borrow_more_than_deposited() external view returns (bool) {
    return pool.totalBorrows() <= pool.totalDeposits();
}
```

## Fuzzing Lifecycle

For each fuzz input, raptor performs this exact sequence:

```
1. CLONE the post-setup state
2. EXECUTE the action sequence (e.g. 1-32 calls)
3. CHECK all property functions
4. RECORD result:
   - New coverage → add to corpus
   - Property returned false → add to objectives (BUG!)
   - Revert during action → normal execution (not a bug)
5. RESET state (discard clone, go back to base)
```

## Comparison with Other Fuzzers

| Feature          | Raptor                  | Foundry (invariant) | Medusa                 | Echidna                 |
| ---------------- | ----------------------- | ------------------- | ---------------------- | ----------------------- |
| Setup            | `constructor`/`setUp()` | `setUp()`           | Deployment + `setUp()` | `constructor`/`setUp()` |
| Actions          | All external/public     | Handlers            | All external/public    | All external/public     |
| Property prefix  | `property_`             | `invariant_`        | `property_`            | `echidna_`              |
| Property args    | None                    | None                | None                   | None                    |
| Property returns | `bool`                  | `bool`              | `bool`                 | `bool`                  |
| Bug on           | Property `false`        | Invariant `false`   | Property `false`       | Property `false`        |
| Bug on revert    | No                      | No                  | No                     | No                      |

## Configuration

Property prefixes and other behavior can be configured in `raptor.toml`:

```toml
[fuzzing.testing.property]
enabled = true
prefixes = ["property_"]

[fuzzing.testing.assertion]
enabled = true
prefixes = ["assert_"]
```
