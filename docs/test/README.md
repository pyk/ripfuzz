# `ripfuzz test`: Find Failed Assertions

> Status: DRAFT

`ripfuzz test` compiles a single Solidity harness file, deploys it as a test
contract on a sandbox chain, runs its optional `setup()`, and fuzzes for failed
assertions. Use it when you want to know *which assertions can break*, not just
*how bad a value can get*.

```bash
ripfuzz test path/to/Contract.sol
ripfuzz test path/to/Contract.sol:Contract
```

The fuzzer generates stateful sequences of your handler functions, checks
Solidity `assert` panics (`Panic(0x01)`) both inside the handlers and inside
`invariant_*` functions, and reports every distinct failed assertion with the
shortest sequence that reproduces it. Other reverts are plain control flow
(e.g. `require` guards) and never reported.

## When to use `test` vs `max`

| Question                              | Command                  | Harness declares |
| :------------------------------------ | :----------------------- | :--------------- |
| Which assertions can fail, and where? | `ripfuzz test <harness>` | `invariant_*`    |
| What is the largest reachable value?  | `ripfuzz max <harness>`  | `value()`        |

Typical `test` use cases:

- **Protocol invariants.** Check accounting identities, conservation of assets,
  and access-control boundaries after arbitrary handler sequences.
- **Assertion hunting.** Find reachable `assert` failures inside handler
  bodies, not only in dedicated invariant functions.
- **Regression proofing.** Keep the corpus between runs, so a campaign replays
  every previously discovered path and reports regressions fast.

If you need a number ("what is the highest value that can be reached?"), use
`ripfuzz max`. If you need the list of broken assertions, use `ripfuzz test`.

## Writing a test harness

A test harness is a normal Solidity contract with an optional constructor, an
optional `setup()`, optional `summary()`, optional `invariant_*` functions, and
one or more handler functions:

```solidity
// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

contract VaultHarness {
    uint256 public total;

    function setup() external {
        // Optional. Runs once after deployment, before fuzzing.
    }

    function deposit(uint256 amount) external {
        // Handler. Any external/public function that is not `setup`,
        // `summary`, or `invariant_*` is fuzzed.
        total += amount;
    }

    function invariant_total_nonzero() external view {
        // Invariant. Appended after every handler call.
        assert(total < type(uint256).max);
    }

    function summary() external {
        // Optional. Runs after the campaign on the final state.
    }
}
```

Rules:

- The contract name defaults to the file stem. Use `path/to/File.sol:Name` when
  the file contains several contracts and you want a different one.
- The constructor must take no arguments and must not be `payable`.
- `setup`, `summary`, and every `invariant_*` must take no arguments and must
  not be `payable`.
- Handler calls revert all the time (failed `require`, custom errors); that is
  expected. Only a `Panic(0x01)` from a failed `assert` is a finding.

Semantics:

- **Invariants are checks, not calls.** After each committed handler call,
  every `invariant_*` function runs on a throwaway clone of the current state.
  Invariant state changes are never committed, invariants never consume
  `--max-calls`, and a panic there is reported with the sequence that produced
  the state.
- **Findings are distinct assertions.** Findings are deduplicated by the
  panicking function and the revert output, so the campaign reports each
  distinct failed assertion once, no matter how many paths reach it.
- **Shrinking preserves the panic.** Every finding's sequence is shrunk in
  parallel; a candidate is accepted only when a clean-state replay still panics
  with the exact same revert output.

## Running it

```bash
# Bare file or fully-qualified artifact id
ripfuzz test VaultHarness.sol
ripfuzz test src/VaultHarness.sol:VaultHarness

# Common options (same as `ripfuzz max`)
ripfuzz test src/VaultHarness.sol:VaultHarness \
  --max-runs 50000 \
  --max-calls 32 \
  --threads 8 \
  --timeout 300 \
  --max-failures 64 \
  --corpus-dir ./corpus
```

Key flags:

| Flag             | Default           | Meaning                                   |
| :--------------- | :---------------- | :---------------------------------------- |
| `--max-runs`     | `256`             | Total sequences fuzzed across all threads |
| `--max-calls`    | `8`               | Max handler calls per sequence            |
| `--threads`      | `1`               | Parallel fuzzer workers                   |
| `--timeout`      | none              | Wall-clock timeout for fuzzing            |
| `--max-failures` | `256`             | Distinct failed assertions to collect     |
| `--corpus-dir`   | `.ripfuzz/corpus` | Where interesting sequences are persisted |

Exit code is `0` even when assertions fail. Findings are results, not campaign
errors. The command fails only on harness validation errors, a failed
deployment, or a failed `setup`.

## Understanding the result

A run logs each phase:

```text
harness deployed at 0x...
setup executed for ... at 0x...
loading corpus .ripfuzz/corpus/...
replaying 17 corpus entries
corpus loaded & replayed
fuzzing started: 1 thread, 10000 runs, max 8 calls, 0 invariants, no timeout
new broken invariant GATED-BYTES32
fuzzing finished: 11 broken invariants, 10000 runs, 0s
shrinking started: 11 broken invariants, 1 thread, 10000 runs
broken invariant GATED-BYTES32 minimized from 7 calls to 1
shrinking finished: 11 broken invariants, 0s
corpus saved: 17 entries to .ripfuzz/corpus/...
broken invariant GATED-BYTES32 saved to .ripfuzz/traces/...
```

What to look at:

- **`new broken invariant`**: the `rvm.bail` id, emitted once per distinct
  finding.
- **Shrunk sequences**: the minimal handler calls that still reproduce each
  broken invariant, replayed with tracing at the end of the campaign so the
  console shows the logs emitted on the way to the failure.
- **Trace files**: one per finding under `.ripfuzz/traces`, plus the optional
  `summary` run when no broken invariant was found.
- **Coverage report**: `lcov.info` with line and function hits for the whole
  campaign, written to `.ripfuzz/coverage` at the end of the run.
- **Log file**: full campaign log under `.ripfuzz/logs`.
- **Corpus**: interesting sequences persist between campaigns, so the next run
  starts from known paths instead of rediscovering them.

## See also

- [Harness Contract Reference](../harness-contract.md): full harness model,
  handlers, setup, summary, cheatcodes
- [Fork Mode](../fork-mode.md): multi-fork isolation and harness storage
  sharing
- [Glossary](../glossary.md): campaign, fuzzer, shrinker, coverage terms
- [`ripfuzz max`](../max/README.md): maximize a harness value instead of
  checking assertions
