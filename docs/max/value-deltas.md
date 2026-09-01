# Value Deltas for the `max` Search

> Status: shipped

`ripfuzz max` measures `value()` after every handler call and uses the per-call
deltas to guide the search. The layer is deliberately minimal: one view call
per handler call, no new plumbing concepts, no harness changes.

The DRLVaultV3 challenge is the running example. Its exploit is a ladder:

1. `reducePrice` dumps USDC into the pool and crashes the price
2. `DRLVaultV3.swapToWETH` swaps the vault's 100k USDC at the crashed price,
   donating roughly 100k USDC of value into the pool
3. `increasePrice` unwinds the attacker's WETH and captures the donation

Measured with `value()` after each call, the trajectory is:

```text
baseline                  5008 ETH
after reducePrice          923 ETH   delta -4085
after swapToWETH           923 ETH   delta     0
after increasePrice       5035 ETH   delta +4112
```

The in-repo `Ladder` and `LadderWithNoise` challenges replay that same 5008 to
923 to 923 to 5035 path. `make challenges` reaches 5035 on both.

## The measurement

The worker executes the sequence and calls `value()` after every call:

```text
[call_1, value(), call_2, value(), call_3, value()]
```

This yields the trajectory `V0, V1, ..., Vn`, where `V0` is the baseline
measured after setup and `V_i` is the value after the first `i` calls. The
delta of call `i` is `d_i = V_i - V_{i-1}`.

Cost. `value()` is a view function that reads a handful of balances. A view
exec is far cheaper than a typical handler call, which runs swaps, router hops,
or accounting. The layer at most doubles the number of EVM calls in a run and
adds a small fraction of the total gas. That price buys the only thing a search
over stateful sequences needs: knowing which call moved the objective, and by
how much.

## Why deltas: profit is not monotonic

Exploit trajectories usually dip before they pay out. The DRLVaultV3 trajectory
collapses by 4000 ETH on the way in and recovers with a profit on the way out.
Two rules follow.

- **Never prune on current value.** A prefix that is far below the baseline may
  be the mandatory entry point of the exploit. Ranking prefixes by their
  current value kills the dip and loses the exploit.
- **A large negative delta is evidence, not damage.** Value moved, so the state
  changed economically. The size of the dip is a measurable signal about how
  aggressively the state was changed, available even when the call reverts or
  when the value never comes back.

## The delta taxonomy

Each delta is classified, and each class drives a different mechanism:

| Signal             | Meaning                                      | Fuzzer action                                                           |
| :----------------- | :------------------------------------------- | :---------------------------------------------------------------------- |
| `d > 0`            | the call gained value                        | feeds the best value climb, per prefix instead of per sequence          |
| `d < 0`            | the call spent or moved value                | never prune, candidate corpus entry on records                          |
| new record `min d` | the largest drop ever seen for this class    | admit the prefix to the corpus                                          |
| new record `max d` | the largest gain ever seen for this class    | admit the prefix to the corpus, weight up                               |
| recovery           | the trajectory is below baseline and `d > 0` | admit the prefix, a call that climbs out of a dip is on an exploit path |

A class is a `(prefix signature, final handler)` pair. The prefix signature is
the sequence of handler selectors before the call, which is cheap to compute
and stable across runs. Records are one `min` and one `max` per class, not a
growing history.

The records are the poor man's potential estimate. Instead of learning
`P(future profit | state)` with a model, the fuzzer keeps the extreme deltas
per class and treats a new extreme as novelty. A call that sets a new record
changed the objective more than any call of its kind before it, which is
exactly the state a search should keep exploring from.

## Where each signal plugs in

The worker interest check gains delta clauses next to coverage and value
climbs:

```rust
let interesting = update.is_interesting()
    || improved
    || beats_local
    || delta.new_record_min
    || delta.new_record_max
    || delta.recovery;
```

`corpus.random_base` weights entries by delta activity, so prefixes that moved
the objective get extended more often than prefixes that did nothing. Activity
is zero for a flat call and otherwise scales with the magnitude bit length, so
a large dump is drawn more often than a one-wei nudge without ranking the
current value.

When a sequence sets a new best, **all of its prefixes become corpus entries**,
each carrying its measured value. This retroactive retention is the payoff of
measuring the full trajectory: the fuzzer learns not just which sequence won,
but which of its prefixes built toward the win, and it keeps every one of them
for future extension.

A later potential score for prefix ranking is not shipped:

```text
score(s) = value(s) + lambda * estimated_future_profit(s)
```

with
`(last delta, cumulative delta, min delta, max delta, distance below baseline)`
as the feature vector. The challenge suite did not demand it. Revisit only if a
ladder appears that activity weighting cannot keep alive.

## Known limitation: the flat rung

Delta signals only see movement in the measured wallet. The DRLVaultV3 middle
rung has `d = 0`: the vault donated 100k USDC to the pool and the attacker
wallet is unchanged, so no delta class fires on the state the ladder needs.
`Ladder.swap` is the same shape.

The rung is not lost, because the interest check is a disjunction. Three
admission paths apply:

- **Coverage.** `update.is_interesting()` is part of the check, so the flat
  rung joins the corpus whenever the flat call exercises code the campaign has
  not seen from earlier states: extreme price branches, partial fill paths, a
  first time external call. Because the record rungs keep the dip base alive
  from the moment it is first sampled, the flat extension is attempted early,
  inside the window where coverage is still fresh
- **Retroactive retention.** After the first win, the winning sequence's
  prefixes carry their measured values into the corpus, the flat rung among
  them, and the next campaign extends the middle state directly
- **Delta records on the surrounding rungs.** `min d` on `reducePrice` and the
  recovery on `increasePrice` keep the ladder's ends alive even when the middle
  rung itself admits nothing

Whether coverage fires on the flat rung is exploit specific. A flat call that
re-executes fully covered code with a zero delta admits nothing on its own, and
for DRLVaultV3 the vault swap covers most of its edges from the initial state.
The design degrades gracefully: it uses the coverage path when it exists, and
retroactive retention when it does not.

A zero delta is also genuinely ambiguous: an approval is a flat call that
enables later profit, while a failed call is flat because nothing happened. The
layer treats zero deltas as neutral: neither pruned nor admitted by delta
alone. Enabler detection needs a different signal than value, and is out of
scope here.

## Design rules

- Value every delta exactly the way `value()` values the objective, so deltas
  and the oracle stay comparable
- Never prune on negative deltas, treat zero deltas as neutral
- Keep one `min` and one `max` record per class, never a history
- Keep classes cheap: selector signatures, not state contents
- Measure on the same chain the calls ran on, so the trajectory reflects the
  sequence exactly as executed
- Record novelty resets between campaigns unless the corpus file grows a
  section for it

## Decisions

- **Classes are `(prefix signature, final handler)`.** Finer classes shipped
  with the layer. The Ladder suite did not need coarser "final handler only"
  records.
- **No potential score.** Corpus sampling uses delta activity instead of
  `value + lambda * future profit`. Lambda and depth decay are not open: they
  only matter if that score is added, and the challenge suite did not demand
  it. Revisit only if a ladder appears that activity weighting cannot keep
  alive.
- **Records do not retire mid-campaign.** An early extreme stays the bar for
  the rest of the campaign. Later near-matches are not admitted by record
  novelty, and that is intended: one `min` and one `max` per class, never a
  history. Novelty resets between campaigns unless the corpus file grows a
  section for it.
