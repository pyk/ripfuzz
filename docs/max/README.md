# `ripfuzz max`: Maximize a Harness Value

> Status: DRAFT

`ripfuzz max` searches for the call sequence that **maximizes** a single
`uint256` value returned by your harness. Use it when you want to know *how
bad* something can get, not just *whether* an invariant breaks.

```bash
ripfuzz max MyHarness
ripfuzz max src/MyHarness.sol:MyHarness
```

The fuzzer generates stateful sequences of your handler functions, evaluates
your `value` function over those sequences, and keeps the highest value it has
ever seen plus the shortest sequence that produced it. At the end it shrinks
that sequence and reports the maximum with a trace you can replay.

## When to use `max` vs `test`

| Question                              | Command                  | Harness declares |
| :------------------------------------ | :----------------------- | :--------------- |
| Does any sequence break an assertion? | `ripfuzz test <harness>` | `invariant_*`    |
| What is the largest reachable value?  | `ripfuzz max <harness>`  | `value()`        |

They are mutually exclusive. A harness with a `value` function cannot also
declare `invariant_*` functions. Ripfuzz fails fast if either rule is violated.

Typical `max` use cases:

- **Value maximization.** Measure the highest reachable value for a
  protocol-defined metric, such as total assets withdrawable in the fuzzed
  state, to validate that limits and caps hold.
- **Worst-case accounting.** Maximize debt, surplus, or accounting imbalance to
  prove solvency and conservation properties.
- **Resource bounds.** Maximize gas used, storage growth, or queue length to
  validate DoS resistance and resource limits.
- **Bound search.** Maximize a balance that should never exceed a threshold to
  show the threshold is tight and never violated.

If you only need a yes/no answer ("does this ever revert with `assert`?"), use
`ripfuzz test`. If you need a number ("what is the highest value that can be
reached?"), use `ripfuzz max`.

## Writing a max harness

A max harness is a normal Solidity contract with three kinds of functions:
setup, handlers, and one value function. It reuses the same harness model as
`ripfuzz test`. Only the checked function changes.

### 1. The `value` function

```solidity
function value() external view returns (uint256) {
    return token.balanceOf(account) - startBalance;
}
```

Rules:

- Name must be exactly `value`.
- Takes **no arguments**.
- Returns **exactly one `uint256`**.
- Must be `view` or `pure`.
- No `invariant_*` functions alongside it.

Semantics:

Higher is always better. Ripfuzz keeps the highest value seen and the shortest
sequence that produced it.

Optional `summary()` is still supported. It is called once after shrinking in
the traced re-run so you can log a final summary that appears in the trace.

### 2. Handlers and setup

Same as `ripfuzz test`. See [harness-contract.md](../harness-contract.md).

- **Setup:** the constructor plus an optional `setup()` function. Runs once,
  then the state is cloned for every fuzz input. Put forks and label setup
  here.
- **Handlers:** every `external`/`public` function that is not `setup`,
  `value`, or `summary`. Ripfuzz calls these with mutational, coverage-guided
  arguments in sequences up to `--max-calls` (default `32`).

## Running it

```bash
# Bare name or fully-qualified artifact id
ripfuzz max VaultHarness
ripfuzz max src/VaultHarness.sol:VaultHarness

# Common options (same as `ripfuzz test`)
ripfuzz max src/VaultHarness.sol:VaultHarness \
  --max-runs 50000 \
  --max-calls 64 \
  --threads 8 \
  --seed 42 \
  --corpus-dir ./corpus

# Fork-aware run (RPC comes from the harness, not the CLI)
ETH_RPC_URL=https://eth.example.com ripfuzz max src/VaultHarness.sol:VaultHarness
```

Key flags:

| Flag               | Default    | Meaning                                   |
| :----------------- | :--------- | :---------------------------------------- |
| `--max-runs`       | `10000`    | Total fuzz iterations across all threads  |
| `--max-calls`      | `32`       | Max handler calls per sequence            |
| `--threads`        | cores      | Parallel fuzzer workers                   |
| `--timeout`        | none       | Wall-clock timeout for fuzzing            |
| `--shrink-runs`    | `10000`    | Iterations to minimize the best sequence  |
| `--shrink-threads` | `threads`  | Parallel shrink workers                   |
| `--seed`           | random     | Seed printed at start for replay          |
| `--corpus-dir`     | auto       | Where interesting sequences are persisted |
| `--gas-limit`      | `12500000` | Gas per generated transaction             |

Exit code is `0` on success. `max` never fails on a high value. The maximum is
a result, not an error. The campaign fails only on harness validation errors
(wrong `value` signature, mixed `value` + `invariant_*`, or a
`--stop-on-revert` revert).

## Understanding the result

A successful run logs:

```text
[setup] measured value return base_score=0
[maxxing] started threads=8
[maxxing] progress ... best_score=1230000000000000000
[maxxing] finished best_score=1500000000000000000
[shrink] shrinking max value initial_calls=47
[shrink] shrank max initial_calls=47 final_calls=3
[trace] fulltrace-max-1.log
```

What to look at:

- **`best_score`**: the maximum `uint256` found. Compare it to the base score.
  `0` means nothing beat the initial state.
- **Shrunk sequence**: the minimal handler calls that still achieve
  `best_score`. Replayed verbatim in the trace.
- **Trace file**: `fulltrace-max-1.log` under the campaign output directory,
  plus the campaign log file path printed at the end. Use it to see every call,
  return value, and log.
- **Corpus**: the shrunk best item is written to the corpus so the next run
  replays it immediately.
- **Coverage report**: `lcov.info` with line and function hits for the whole
  campaign.

If the value function reverts until some state is set (for example
`require(value != 0)`), that is fine. Reverted scores are `0`, so the fuzzer
must first discover a sequence that makes the call succeed.

## Reference: `value` rules

- Name `value`, no arguments, single `uint256` return, `view` or `pure`.
- Cannot coexist with `invariant_*`.
- Revert or empty return is scored as `0`.
- Evaluated across the sequence. The shortest prefix achieving the highest
  score is kept.

## See also

- [Harness Contract Reference](../harness-contract.md): full harness model,
  handlers, setup, summary, cheatcodes
- [Fork Mode](../fork-mode.md): multi-fork isolation and harness storage
  sharing
- [Glossary](../glossary.md): campaign, fuzzer, shrinker, coverage terms
- `ripfuzz test`: stateful invariant testing with `invariant_*` functions
