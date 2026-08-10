# Harness Contract Reference

This document is the reference for writing a Solidity contract that
`ripfuzz run` can fuzz.

## Overview

A ripfuzz harness contract is a normal Solidity contract with **four kinds of
functions**:

1. **Setup Functions**: initialize state before fuzzing begins
2. **Handler Functions**: function calls the fuzzer can make to mutate state
3. **Invariant Functions**: invariants the fuzzer checks after every call
   sequence
4. **Max Functions**: read-only values the fuzzer maximizes

Declaring a `max_*` function automatically puts the harness in max mode;
otherwise ripfuzz runs in invariant mode. The two modes are mutually exclusive:
max mode supports exactly one `max_*` function and rejects harnesses that also
declare `invariant_*` functions.

## Using ripfuzz-std

[ripfuzz-std](https://github.com/pyk/ripfuzz-std) is the standard library for
ripfuzz harnesses. It provides:

- **Harness**: base contract with an `rvm` instance for cheatcodes
- **RVM**: the cheatcode interface ripfuzz implements

A harness does not need to use ripfuzz-std. Plain Solidity is enough for pure
logic. Use ripfuzz-std when you need to set balances, prank callers, warp time,
label addresses, or call other RVM helpers.

### Installation

```bash
forge install pyk/ripfuzz-std
```

### Example with cheatcodes

Inherit from `Harness` to get `rvm` at the ripfuzz VM address
(`keccak256("ripfuzz cheatcode")`):

```solidity
// SPDX-License-Identifier: MIT
pragma solidity 0.8.35;

import {Harness} from "ripfuzz/Harness.sol";

contract Counter {
    uint256 public count;
    address public owner;

    constructor() {
        owner = msg.sender;
    }

    function increment() external {
        require(msg.sender == owner, "not owner");
        count += 1;
    }

    function add(uint256 x) external {
        require(msg.sender == owner, "not owner");
        count += x;
    }
}

contract CounterHarness is Harness {
    Counter counter;
    address user;

    // [+] Setup ==============================================================

    /// @notice Establish the initial world state.
    /// @dev Ripfuzz deploys once, then clones that state for every fuzz input.
    function setup() external {
        user = address(0xBEEF);
        rvm.deal(user, 100 ether);
        rvm.label(user, "user");

        // Deploy as `user` so that user owns the counter.
        rvm.prank(user);
        counter = new Counter();
    }

    // [+] Handler functions ==================================================

    /// @notice Increment the counter as `user`.
    /// @dev Handler functions are external/public and not setup or invariants.
    ///      Ripfuzz calls these with type-appropriate random inputs.
    function increment() external {
        rvm.prank(user);
        counter.increment();
    }

    /// @notice Add `x` to the counter as `user`.
    function add(uint256 x) external {
        rvm.prank(user);
        counter.add(x);
    }

    // [+] Invariant functions ================================================

    /// @notice Count must stay below 1000.
    /// @dev Invariant functions use the `invariant_` prefix and take no
    ///      arguments. Ripfuzz appends them to every call sequence. An
    ///      `assert` panic is reported as a bug.
    function invariant_CountStaysSmall() external view {
        assert(counter.count() < 1000);
    }
}
```

### Available cheatcodes

| Category    | Cheatcodes                                                                                     |
| ----------- | ---------------------------------------------------------------------------------------------- |
| Block       | `warp`, `roll`, `fee`, `coinbase`, `prevrandao`, `chainId`                                     |
| Account     | `deal`, `etch`, `setNonce`, `getNonce`, `store`, `load`                                        |
| Prank       | `prank`, `startPrank`, `stopPrank`                                                             |
| Label       | `label`, `getLabel`                                                                            |
| Conversion  | `toString`, `parseUint`, `parseInt`, `parseBool`, `parseAddress`, `parseBytes`, `parseBytes32` |
| Bytecode    | `getCode`                                                                                      |
| Wallet      | `addr`, `sign`                                                                                 |
| FFI         | `ffi`                                                                                          |
| Environment | `getEnv`                                                                                       |
| Fork        | `fork`                                                                                         |

The full interface lives in
[RVM.sol](https://github.com/pyk/ripfuzz-std/blob/main/src/RVM.sol). More
cheatcodes will be added as ripfuzz grows support.

## 1. Setup Functions

Setup establishes the **base state** that every fuzz input starts from.

### What counts as setup

- The contract **constructor** always runs once at deployment
- A function named **`setup()`**. Ripfuzz calls this automatically after
  deployment if it exists

### Rules

- Setup runs **exactly once** per fuzz campaign
- The resulting state is **cloned** for every fuzz input
- `setup` is not treated as a handler function

### What to put in setup

Use the constructor for simple initialization. Prefer `setup()` when you need
cheatcodes or multi-step deployment after the harness itself exists.

In the CounterHarness example above, `setup()`:

- Funds and labels a user with `rvm.deal` / `rvm.label`
- Deploys the target under that identity with `rvm.prank`
- Stores handles the handlers will use later

## 2. Handler Functions

These are the functions ripfuzz calls with random inputs to explore state
space.

### Discovery rules

A function is treated as a handler function if **all** of these are true:

- It is `external` or `public`
- It is **not** a setup function (`setup`)
- It does **not** match an invariant prefix (see below)

### Input generation

Ripfuzz generates ABI-encoded calldata for each function call by randomly
selecting a handler function and producing values for every argument.

- The 4-byte selector is fixed (from the ABI)

- Arguments are generated by type via `DynSolType::random`:
  - `bool` → true or false (50/50)
  - `uintN` / `intN` → 20% chance to pick a literal extracted from the project,
    30% chance to pick an edge case (`0`, `1`, `max`, `max-1`, `max-2`,
    `max-3`), 50% chance to generate uniformly random bytes
  - `address` → same distribution as above (20% literal, 30% edge, 50% random)
  - `bytes` / `string` → same distribution, with edge cases including empty,
    1-byte, 32-byte, and 64-byte values
  - `bytesN` → same distribution, sized to the exact width
  - arrays, tuples, and structs → recursively generated with lengths 0-4

- For `payable` functions, `msg.value` is generated using the same `uint256`
  distribution

- All literal pools are seeded by scanning the compiled project for concrete
  values (constants, immutables, literals) before the campaign starts

### Multiple function calls per input

A single fuzz input is a **sequence of function calls** (default: up to 32
calls). This lets ripfuzz explore stateful interactions.

```solidity
// A single fuzz input might do:
// 1. deposit(100)
// 2. borrow(50)
// 3. deposit(200)
// 4. borrow(999)   // <- maybe this triggers an invariant violation
```

## 3. Invariant Functions

Invariant functions are **state checks** that must always hold.

### Naming

By default, invariant functions must match the prefix:

```text
invariant_
```

Use PascalCase for the name after the prefix:

```text
invariant_BalancePositive
invariant_NoReentrancy
```

### Signature requirements

A valid invariant function **must** have this signature shape:

```solidity
function invariant_Name() external
```

Requirements:

- Name starts with `invariant_` followed by a PascalCase name
- Takes **no arguments**
- Return type is optional and ignored

Invariant functions need not be declared `view` or `pure`. Emitting events for
debugging is allowed. Ripfuzz runs invariants on cloned state and discards the
clone afterward, so any storage writes are naturally isolated.

### Semantics

- Ripfuzz appends **all** invariant calls to the end of every function call
  sequence and executes them in the same EVM loop
- If **any** call (handler function or invariant) reverts with a Solidity
  `assert` panic (`Panic(0x01)`), the fuzzer records a **failed assertion**
  (objective)
- Reverts caused by `require` or other reasons do **not** produce a failed
  assertion. The sequence continues executing and invariants are still checked
- Invariants are **not** called as handler functions (they are appended, not
  randomly generated)

### Example invariants

```solidity
function invariant_Solvency() external {
    assert(token.balanceOf(address(pool)) >= pool.totalDeposits());
}

function invariant_UserCantBorrowMoreThanDeposited() external {
    assert(pool.totalBorrows() <= pool.totalDeposits());
}
```

### Types of Invariants

Invariants can be divided into two categories based on their scope.

#### Function-Level Invariants

A **function-level invariant** is a property that arises from the execution of
a specific function. It describes what must be true *before* and *after* that
single function runs. For example, after calling `deposit(uint256 amount)`, the
contract's ETH balance should increase by `amount` and the sender's balance
should decrease by the same amount.

In ripfuzz, you can test function-level invariants by adding `assert`
statements directly inside the handler function itself. The fuzzer records a
failed assertion whenever any call reverts with a Solidity `assert` panic
(`Panic(0x01)`), regardless of whether the assertion is in a handler function
or an `invariant_` function.

```solidity
contract CounterHarness {
    uint256 public count;

    function increment() external {
        uint256 before = count;
        count += 1;
        assert(count == before + 1);
    }

    function add(uint256 x) external {
        uint256 before = count;
        count += x;
        assert(count == before + x);
    }
}
```

#### Protocol-Level Invariants

A **protocol-level invariant** is a property that must hold after *any*
sequence of function calls, not just one specific function. These are more
general than function-level invariants. For example:

- The `xy = k` constant product formula should always hold for a Uniswap pool.
- The total deposited amount in a lending protocol should never exceed
  `MAX_DEPOSIT_AMOUNT`.
- No user's balance should exceed the total supply of an ERC20 token.

Protocol-level invariants are the most common use case for ripfuzz's
`invariant_` functions because they are checked automatically after every
function call sequence.

```solidity
contract VaultHarness {
    uint256 public totalDeposits;
    uint256 public constant MAX_DEPOSIT = 1000;

    function deposit(uint256 amount) external {
        totalDeposits += amount;
    }

    function withdraw(uint256 amount) external {
        require(totalDeposits >= amount);
        totalDeposits -= amount;
    }

function invariant_TotalWithinLimit() external {
        assert(totalDeposits <= MAX_DEPOSIT);
    }
}
```

## Max Functions

Max functions turn a harness value into a maximization objective. They must:

- start with the prefix `max_`
- take no arguments
- return a single `uint256`
- be `pure` or `view`

Declaring a `max_*` function puts the harness in max mode automatically:

```solidity
contract ProfitHarness {
    uint256 public assets;
    uint256 public debt;

    function setAssets(uint256 amount) external {
        assets = amount;
    }

    function setDebt(uint256 amount) external {
        debt = amount;
    }

    function max_profit() external view returns (uint256) {
        return assets > debt ? assets - debt : 0;
    }
}
```

```bash
ripfuzz run ProfitHarness
```

Max mode supports exactly one `max_*` function per harness and cannot be
combined with `invariant_*` functions; ripfuzz fails with a clear error if
either rule is violated. It calls the max function after each handler call and
keeps the highest value plus the shortest handler prefix that produced it.
After the campaign it shrinks the best sequence while preserving its value,
reports the maximum value with the call sequence, and writes the result to the
corpus.

Max functions never fail. A value above `0` is the finding: the harness ended
in a state where the maximized quantity is positive (for example attacker
profit after repaying a flash loan).

## Fuzzing Lifecycle

For each fuzz input, ripfuzz performs this exact sequence:

```text
1. CLONE the post-setup state
2. BUILD the call sequence:
   - handler function calls (randomly generated or mutated)
   - invariant calls (appended automatically)
3. EXECUTE every call in a single loop, committing state after each call
   - Succeeded → continue to next call
   - Reverted with assert panic → FAILED ASSERTION (BUG!)
   - Reverted for any other reason → continue to next call
4. RECORD result:
   - New coverage → add to corpus
   - Assert panic detected → add to objectives (BUG!)
   - Normal revert → not a bug
5. RESET state (discard clone, go back to base)
```

## Comparison with Other Fuzzers

| Feature           | Ripfuzz                 | Foundry (invariant) | Medusa                 | Echidna                 |
| ----------------- | ----------------------- | ------------------- | ---------------------- | ----------------------- |
| Setup             | `constructor`/`setup()` | `setup()`           | Deployment + `setup()` | `constructor`/`setup()` |
| Handler Functions | All external/public     | Handlers            | All external/public    | All external/public     |
| Invariant prefix  | `invariant_`            | `invariant_`        | `property_`            | `echidna_`              |
| Invariant args    | None                    | None                | None                   | None                    |
| Invariant returns | Ignored                 | `bool`              | `bool`                 | `bool`                  |
| Bug on            | `assert` panic          | Invariant `false`   | Property `false`       | Property `false`        |
| Bug on revert     | `assert` only           | No                  | No                     | No                      |

## Fork Mode

Campaigns always start as an empty sandbox. Call `rvm.fork` in `setup` (or an
action) to opt into remote state at a pinned block:

```solidity
function setup() external {
    rvm.fork(rvm.getEnv("ETH_RPC_URL"), 21_000_000);
}
```

**Remote state is isolated per fork; harness storage is shared across chains.**
Use harness ghost variables to track value conservation (or other cross-chain
invariants) while each chain keeps its own remote overlay.

Full reference (API, multi-fork model, conservation examples, cache behavior):
[fork-mode.md](./fork-mode.md).

## Configuration

Today the invariant prefix is hardcoded to `invariant_`. It is not yet
configurable at runtime.
