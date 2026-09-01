# `ripfuzz exec`: Run a Script Contract

> Status: DRAFT

`ripfuzz exec` compiles a single Solidity file, deploys it as a script contract
on a sandbox chain, runs its optional `setup()`, and executes `exec()` once.
Use it when you want to replay a stateful on-chain operation end to end and
inspect its logs and execution trace.

```bash
ripfuzz exec path/to/Contract.sol
ripfuzz exec path/to/Contract.sol:Contract
```

## When to use `exec` vs `max`

| Question                                  | Command                 | Contract declares |
| :---------------------------------------- | :---------------------- | :---------------- |
| What does this sequence of operations do? | `ripfuzz exec <script>` | `exec()`          |
| What is the largest reachable value?      | `ripfuzz max <harness>` | `value()`         |

`exec` is deterministic. There is no fuzzing, no corpus, and no search. The
contract runs once, exactly as written, and ripfuzz reports what happened.

Typical `exec` use cases:

- **Replay a position.** Rebuild a live position against a fork, then step
  through the operations you want to inspect.
- **Verify a strategy.** Run a liquidation, arbitrage, or migration flow and
  check the emitted events and balances.
- **Probe onchain state.** Point the script at live contracts, call their
  functions, and inspect storage changes, balances, and events in the trace
  without writing a test harness.
- **Reproduce a finding.** Turn a sequence found by `ripfuzz max` into a fixed
  script you can rerun on demand.

## Writing a script contract

A script contract is a normal Solidity contract with an optional constructor,
an optional `setup()`, and one mandatory `exec()`:

```solidity
// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

contract MyScript {
    function setup() external {
        // Optional. Runs once after deployment, before `exec`.
    }

    function exec() external {
        // Required. Runs once, after deployment and setup.
    }
}
```

Rules:

- The contract name defaults to the file stem. Use `path/to/File.sol:Name` when
  the file contains several contracts and you want a different one.
- `exec` MUST exist, take no arguments, and be `external` or `public`. It must
  not be `payable`.
- `setup` MAY exist. It follows the same rules as `exec`.
- The constructor MAY exist. It must take no arguments and must not be
  `payable`, because ripfuzz deploys with no constructor arguments and no
  value.

## Running it

```bash
# Default contract name comes from the file stem
ripfuzz exec scripts/ClaimAirdrop.sol

# Explicit contract name
ripfuzz exec scripts/ClaimAirdrop.sol:ClaimAirdrop

# Fork-aware run (RPC comes from the script, not the CLI)
ETH_RPC_URL=https://eth.example.com ripfuzz exec scripts/ClaimAirdrop.sol
```

The command:

1. Loads `ripfuzz.toml` for the solc version and output directory.
2. Compiles the script with solc.
3. Creates a sandbox chain, with fork support via `rvm.fork`.
4. Deploys the script contract.
5. Runs `setup()` if the contract defines it.
6. Executes `exec()` once.
7. Prints log output to the console.
8. Saves the execution trace under `.ripfuzz/traces`.

Exit code is `0` when `exec` completes successfully.

## Understanding the result

A successful run logs the deployed address, the log output of `setup` and
`exec`, and the path of the saved trace:

```text
script deployed script=scripts/MyScript.sol:MyScript address=0x…
console log: done
execution trace saved path=/…/.ripfuzz/traces/…-….log
```

Console log output (forge-std style `console.log`) prints directly to the
terminal. Custom events do not print to the console, but they are recorded in
the saved trace.

The saved trace contains every call, event, and storage change of the run,
including deployment and setup. Open it to see exactly what the script did.

Failures fail fast with the failing phase in the message:

```text
script contract `MyScript` deployment failed
script contract `MyScript` setup failed
script contract `MyScript` exec failed
```

Each failure also dumps the execution trace to `.ripfuzz/traces` and prints its
path, so you can see the revert reason and the state at the point of failure.

## See also

- [Harness Contract Reference](../harness-contract.md): the `max`/`test`
  harness model, handlers, setup, summary, cheatcodes
- [Fork Mode](../fork-mode.md): multi-fork isolation and harness storage
  sharing
- `ripfuzz max`: fuzz for the sequence that maximizes a harness value
