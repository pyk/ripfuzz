# Raptor Challenges

A collection of Solidity smart-contract puzzles designed to test
[Raptor](https://github.com/pyk/raptor), a coverage-guided fuzzer.

Each level contains a contract with a `caught()` view function. The fuzzer
succeeds when it discovers an input sequence that makes `caught()` return
`true`. Every wrong move reverts with `unicode"💀"`.

---

## Level 1 — L1SimpleKnob

**File:** `src/L1SimpleKnob.sol`

A three-step sequence lock. The functions must be called in exact order:

1. `one()`
2. `two()`
3. `three()`

Calling any function out of order reverts. Raptor must learn that a fixed
sequence of three calls (ignoring value inputs) leads to success.

**Goal:** `property == 3 ether`

---

## Level 2 — L2ValueGate

**File:** `src/L2ValueGate.sol`

A single function that requires a specific input value:

- `unlock(uint256 key)` reverts unless `key == 0xBAAAAAAD`.

Raptor must discover that a particular 32-byte value, not just any non-zero
value, is needed to pass the gate.

**Goal:** `property == 2 ether`

---

## Level 3 — L3CounterStrike

**File:** `src/L3CounterStrike.sol`

A counter that must be incremented an exact number of times:

- `tick()` — increments an internal counter.
- `claim()` — succeeds only when the counter is exactly `7`.

Raptor must learn that repeating the same call exactly seven times (no more, no
less) before calling `claim()` is the winning strategy.

**Goal:** `property == 3 ether`

---

## Level 4 — L4StateMachine

**File:** `src/L4StateMachine.sol`

A strict state machine where wrong transitions reset progress:

- `stepA()` — valid only from idle.
- `stepB()` — valid only after `stepA`.
- `stepC()` — valid only after `stepB`.
- `finish()` — valid only after `stepC`.

Any mis-ordered call resets the state back to idle. Raptor must find the exact
sequence `A → B → C → finish` without any detours.

**Goal:** `property == 4 ether`

---

## Level 5 — L5ComboLock

**File:** `src/L5ComboLock.sol`

A combination lock that checks both order and value properties:

1. `prime(uint256 n)` — accepts any prime number.
2. `even(uint256 n)` — accepts any even number.
3. `odd(uint256 n)` — accepts any odd number.

Any wrong value or wrong order resets the lock. Raptor must find a valid prime,
then a valid even, then a valid odd number in exactly that order.

**Goal:** `property == 5 ether`

---

## How to fuzz a level

```sh
cd fixtures/challenges
raptor fuzz src/L1SimpleKnob.sol
```

Raptor will compile the contract, deploy it, and run the fuzz loop. If it
catches the dragon, the fuzzing session will report crashes that lead to
`caught() == true`.
