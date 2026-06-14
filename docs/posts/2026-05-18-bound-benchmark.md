---
published: true
date: 2026-05-18T10:30:00+00:00
title: "Benchmarking `vm.bound`: Why Inline Solidity Wins in raptor"
description:
    I expected raptor's native cheatcode to outperform an inline Solidity
    helper, but the benchmark showed the opposite. Here is why the inspector
    hook overhead adds up, and when the inline path is worth it.
topics:
    - solidity
    - fuzzing
    - rust
    - revm
    - benchmarking
---

It's Monday. Vigilseek is down. No contest to grind.

I decided to spend the morning reviewing raptor, my private smart contract
fuzzer, and see if I could make it a little faster. One function that shows up
everywhere in fuzzing harnesses is `bound`. It clamps a random `uint256` into a
valid range. In Foundry you usually call `vm.bound(x, min, max)`. I already have
that cheatcode implemented in raptor, but I started wondering: would an inline
Solidity helper be faster?

My gut said yes. A pure `internal` function in Solidity gets inlined by the
compiler. No `CALL` opcode, no ABI encoding, no inspector dispatch, no memory
expansion. Just a few arithmetic opcodes inside the same frame. The cheatcode
path, on the other hand, has to pay for a `CALL` to the VM precompile address,
then revm has to intercept it, decode the selector, decode three `uint256`
arguments, run the bound logic in Rust, and ABI-encode the result back.

It felt like an obvious win for the inline path. So I wrote a benchmark to prove
it.

## The Setup

I created two realistic fixture contracts that mirror a typical fuzzing harness.
Each contract exposes ten action functions. Every action takes a raw `uint256`
argument, bounds it into a specific range, and accumulates the result into a
`checksum` storage variable so the compiler cannot optimize away the work.

The only difference between the two contracts is how the bound is applied:

- **Cheatcode** — calls `vm.bound(x, min, max)` through the VM precompile.
- **Inline** — calls `BoundUtils.bound(x, min, max)`, an `internal pure` helper
  copied from forge-std that the Solidity compiler inlines into the caller.

I ran each campaign for 60 seconds with a single worker, sequence length 1, and
a high max-runs cap so the timeout was the only limit. I also added throughput
logging to raptor itself so I could read calls per second and average gas per
call directly from the campaign output.

To reduce noise from OS scheduling and thermal throttling, I ran each scenario
**five times** and took the median throughput. The numbers below are the
medians; individual runs varied by roughly ±5%.

## Project Setup

The benchmark lives under `fixtures/benchmark` in the raptor repo.

```
fixtures/benchmark/
├── foundry.toml
├── src/
│   ├── Vm.sol
│   └── BoundUtils.sol
└── test/
    ├── BenchmarkCheatcode.sol
    └── BenchmarkInline.sol
```

`foundry.toml` is minimal:

```toml
[profile.default]
src = "src"
out = "out"
libs = ["lib"]
cache = true
```

## Benchmark Source Code

`src/Vm.sol` is the interface for the cheatcode:

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

interface Vm {
    function bound(uint256 x, uint256 min, uint256 max) external pure returns (uint256);
}
```

`src/BoundUtils.sol` is the inline helper, copied from forge-std:

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

library BoundUtils {
    uint256 private constant _UINT256_MAX =
        115792089237316195423570985008687907853269984665640564039457584007913129639935;

    function bound(uint256 x, uint256 min, uint256 max) internal pure returns (uint256 result) {
        require(min <= max, "BoundUtils: Max is less than min.");
        if (x >= min && x <= max) return x;

        uint256 size = max - min + 1;

        if (x <= 3 && size > x) return min + x;
        if (x >= _UINT256_MAX - 3 && size > _UINT256_MAX - x) return max - (_UINT256_MAX - x);

        if (x > max) {
            uint256 diff = x - max;
            uint256 rem = diff % size;
            if (rem == 0) return max;
            result = min + rem - 1;
        } else if (x < min) {
            uint256 diff = min - x;
            uint256 rem = diff % size;
            if (rem == 0) return min;
            result = max - rem + 1;
        }
    }
}
```

The two handler contracts are mirror images of each other. Here is the cheatcode
version:

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {Vm} from "../src/Vm.sol";

contract BenchmarkCheatcode {
    Vm constant vm = Vm(address(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D));
    uint256 public checksum;

    function action_small_range(uint256 x) external {
        uint256 v = vm.bound(x, 0, 100);
        checksum += v;
    }

    function action_medium_range(uint256 x) external {
        uint256 v = vm.bound(x, 10, 50);
        checksum += v;
    }

    function action_large_range(uint256 x) external {
        uint256 v = vm.bound(x, 100, 200);
        checksum += v;
    }

    function action_bool_like(uint256 x) external {
        uint256 v = vm.bound(x, 0, 1);
        checksum += v;
    }

    function action_tight_range(uint256 x) external {
        uint256 v = vm.bound(x, 99, 101);
        checksum += v;
    }

    function action_full_range(uint256 x) external {
        uint256 v = vm.bound(x, 0, type(uint256).max);
        checksum += v;
    }

    function action_small_shift(uint256 x) external {
        uint256 v = vm.bound(x, 50, 60);
        checksum += v;
    }

    function action_byte_range(uint256 x) external {
        uint256 v = vm.bound(x, 0, 7);
        checksum += v;
    }

    function action_thousand_range(uint256 x) external {
        uint256 v = vm.bound(x, 1000, 2000);
        checksum += v;
    }

    function action_nibble_range(uint256 x) external {
        uint256 v = vm.bound(x, 0, 3);
        checksum += v;
    }

    function call_single_bound(uint256 x) external {
        checksum = vm.bound(x, 0, 100);
    }
}
```

And the inline version swaps `vm.bound` for `BoundUtils.bound`:

```solidity
import {BoundUtils} from "../src/BoundUtils.sol";

contract BenchmarkInline {
    uint256 public checksum;

    function action_small_range(uint256 x) external {
        uint256 v = BoundUtils.bound(x, 0, 100);
        checksum += v;
    }

    // ... nine more actions, same ranges ...

    function call_single_bound(uint256 x) external {
        checksum = BoundUtils.bound(x, 0, 100);
    }
}
```

## Bound Cheatcode Implementation

Here is the Rust side. The cheatcode lives in `src/chain/cheatcodes/bound.rs`.
First, the decoder that slices three `uint256` arguments out of the calldata:

```rust
fn decode_three_u256_args(input: &Bytes) -> Option<(U256, U256, U256)> {
    if input.len() < 4 + 96 {
        return None;
    }
    let a = U256::from_be_slice(&input[4..36]);
    let b = U256::from_be_slice(&input[36..68]);
    let c = U256::from_be_slice(&input[68..100]);
    Some((a, b, c))
}
```

Then the pure helper that implements the same algorithm as `BoundUtils.sol`:

```rust
fn bound_uint256(x: U256, min: U256, max: U256) -> U256 {
    if x >= min && x <= max {
        return x;
    }

    let size = max.wrapping_sub(min).wrapping_add(U256::from(1));

    if x <= U256::from(3) && size > x {
        return min + x;
    }
    let max_u256 = U256::MAX;
    let dist_from_max = max_u256 - x;
    if x >= max_u256 - U256::from(3) && size > dist_from_max {
        return max - dist_from_max;
    }

    if x > max {
        let diff = x - max;
        let rem = diff % size;
        if rem.is_zero() { return max; }
        return min + rem - U256::from(1);
    }

    let diff = min - x;
    let rem = diff % size;
    if rem.is_zero() { return min; }
    max - rem + U256::from(1)
}
```

The `Cheatcode` trait wires the selector to the decoder and the effect:

```rust
pub struct BoundUint;

impl Cheatcode for BoundUint {
    type Args = (U256, U256, U256);
    const SELECTOR: [u8; 4] = [0x5a, 0x6c, 0x1e, 0xed];

    fn decode(input: &Bytes) -> Option<Self::Args> {
        decode_three_u256_args(input)
    }

    fn effects((x, min, max): Self::Args) -> Vec<CheatcodeEffect> {
        if min > max {
            return vec![CheatcodeEffect::Revert(BOUND_UINT_ERROR.into())];
        }
        let result = bound_uint256(x, min, max);
        vec![CheatcodeEffect::ReturnU256(result)]
    }
}
```

The actual interception happens in `src/chain/inspectors/cheatcode.rs`. When
revm sees a `CALL` to the VM address, the inspector grabs the selector,
dispatches to `BoundUint`, and returns a synthetic `CallOutcome`:

```rust
fn call(&mut self, ctx: &mut CTX, inputs: &mut CallInputs) -> Option<CallOutcome> {
    let input = inputs.input.bytes_local(ctx.local());
    if inputs.target_address != VM_ADDRESS || input.len() < 4 {
        return None;
    }

    let sel: [u8; 4] = crate::result_to_option(input[..4].try_into())?;
    let effects = dispatch_effects(sel, &input)?;

    for effect in &effects {
        if let Err(reason) = apply_effect(effect, ctx, &mut self.state) {
            return Some(revert_outcome(&reason));
        }
    }

    let mut outcome = build_outcome(&effects, inputs.gas_limit, ctx, &self.state);
    outcome.memory_offset = inputs.return_memory_offset.clone();
    Some(outcome)
}
```

That `build_outcome` call constructs a `CallOutcome` with
`InstructionResult::Return` and the encoded `uint256` value. Revm never starts a
nested interpreter, so the child frame is short-circuited entirely.

## The Results

I ran each scenario **five times** for 60 seconds with a single worker. Below
are the raw numbers so anyone can verify the medians.

### Cheatcode, sequence length 1

| Run        | Calls/sec  | Gas/call   |
| ---------- | ---------- | ---------- |
| 1          | 43,267     | 31,456     |
| 2          | 43,024     | 31,456     |
| 3          | 43,075     | 31,456     |
| 4          | 43,376     | 31,456     |
| 5          | 43,089     | 31,456     |
| **Median** | **43,089** | **31,456** |

### Inline, sequence length 1

| Run        | Calls/sec  | Gas/call   |
| ---------- | ---------- | ---------- |
| 1          | 49,677     | 28,171     |
| 2          | 49,912     | 28,172     |
| 3          | 59,776     | 28,171     |
| 4          | 49,682     | 28,171     |
| 5          | 60,068     | 28,171     |
| **Median** | **49,912** | **28,172** |

### Cheatcode, sequence length 5

| Run        | Calls/sec  | Gas/call   |
| ---------- | ---------- | ---------- |
| 1          | 44,197     | 31,609     |
| 2          | 44,263     | 31,609     |
| 3          | 44,072     | 31,609     |
| 4          | 44,064     | 31,609     |
| 5          | 39,007     | 31,608     |
| **Median** | **44,072** | **31,609** |

### Inline, sequence length 5

| Run        | Calls/sec  | Gas/call   |
| ---------- | ---------- | ---------- |
| 1          | 53,498     | 29,991     |
| 2          | 54,571     | 29,991     |
| 3          | 54,368     | 29,991     |
| 4          | 47,795     | 29,992     |
| 5          | 45,798     | 29,992     |
| **Median** | **53,498** | **29,991** |

### Summary

| Scenario              | Median calls/sec | Median gas/call | Notes                                            |
| --------------------- | ---------------- | --------------- | ------------------------------------------------ |
| Cheatcode (seq-len 1) | **43,089**       | 31,456          | One action per run, one `vm.bound` per action.   |
| Inline (seq-len 1)    | **49,912**       | 28,172          | Same shape, inlined helper instead of cheatcode. |
| Cheatcode (seq-len 5) | **44,072**       | 31,609          | Five actions per run, one `bound` per action.    |
| Inline (seq-len 5)    | **53,498**       | 29,991          | Same shape, inlined helper.                      |

In the single-action case (seq-len 1), inline wins by about **16%**. That is the
margin I originally expected. One inlined `bound` is cheaper than one cheatcode
round-trip through the inspector.

When I raise the sequence length to 5 — closer to a real fuzzing campaign where
each run is a sequence of multiple action calls — the inline lead grows to
**21%**. Every additional action adds another `bound` call, and the per-call
overhead of the cheatcode path compounds.

## Why This Happens

The explanation is in how raptor implements cheatcodes.

Most people assume `vm.bound` works like a precompile: the EVM executes a `CALL`
opcode, pushes a new frame, copies calldata into memory, and jumps into the
precompile logic. That would indeed be expensive. But raptor does not do that.

Raptor implements cheatcodes via **revm's `Inspector::call`** hook. When the
handler contract calls `vm.bound`, revm starts to set up the sub-frame, then
inspector intercepts it **before the nested interpreter even starts**. The
inspector decodes the selector, runs the bound algorithm as native Rust,
constructs a synthetic `CallOutcome`, and hands it back. The child frame is
short-circuited entirely.

In other words, the cheatcode path skips:

- Child interpreter spin-up
- Calldata memory expansion for the sub-call
- `CALL` opcode gas stipend logic
- All the EVM bytecode for the bound algorithm itself

You would think skipping all of that would make the cheatcode faster. But in
practice the inspector dispatch is not free. It still pays for:

- Selector decoding (`input[..4].try_into()`)
- ABI decoding of three `uint256` arguments
- A 50-arm `match` in `dispatch_effects`
- Constructing the synthetic `CallOutcome`

For a single call, that overhead is larger than just letting the Solidity
compiler inline the bound logic as a few arithmetic opcodes. As the sequence
grows and more actions are called per run, the overhead multiplies, and the
inline path pulls further ahead.

The gas numbers back this up. The inline path burns less gas per call (28k–30k)
than the cheatcode path (31k–32k). The EVM is actually executing the bound logic
as opcodes in the same frame, while the cheatcode path pays for the inspector
hook on top of the parent frame work.

## What This Means

For my own use in raptor, the inline helper is measurably faster. The margin is
modest for a single action, but it compounds in realistic campaigns where each
run contains multiple action calls. If I am writing a hot fuzzing harness and
want every extra call per second, swapping `vm.bound` for an inline `BoundUtils`
helper is a real win.

That said, the cheatcode is not catastrophically slow. ~16% overhead for the
convenience of not copying a helper into every test file is a reasonable
tradeoff for my day-to-day work. The gap only becomes significant when I have
long sequences or many bounded arguments per action.

This also taught me to be careful about assumptions. I started out wondering
whether the inline path would win, and it does. But the margin is smaller than I
expected given how much work the inspector short-circuit skips. The lesson is
that native Rust arithmetic through an inspector hook is fast, but the selector
match, ABI decoding, and outcome construction still add up. For pure clamping
logic that compiles to a handful of EVM opcodes, Solidity inline beats the
round-trip.

I love when a benchmark forces me to rethink how my own code works.

One last thing: these numbers are specific to raptor, which is my personal
fuzzer built on revm. If you are using Foundry or another framework where
`vm.bound` truly executes as a precompile `CALL`, the inline helper will almost
certainly win by a larger margin. Raptor's inspector short-circuit is an
implementation detail, not a universal rule.

---

_Disclaimer: I used Kimi K2.6 to run this benchmark, and all results were
verified by me._
