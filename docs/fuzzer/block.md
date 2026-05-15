# Block in Fuzzing Campaigns

This document explains how Medusa models block numbers and timestamps during
stateful fuzzing, and why it matters for finding bugs in Solidity contracts.

## Why Block Numbers Matter

Solidity contracts often depend on `block.number` or `block.timestamp`:

- **Time locks**: `require(block.timestamp > unlockTime)`
- **Auction deadlines**: `require(block.number < endBlock)`
- **Block-based randomness**: `keccak256(abi.encodePacked(block.number))`
- **Vesting schedules**: release tokens based on block height

If a fuzzer always executes every call at block `0`, it can never explore states
that depend on time passing or blocks advancing. Medusa solves this by giving
each call in a sequence an explicit **block delay**.

## Medusa's Model: Per-Call Delays

Every call in a Medusa call sequence carries two delay fields:

```go
type CallSequenceElement struct {
    // ...
    BlockNumberDelay    uint64  // advance this many blocks
    BlockTimestampDelay uint64  // advance this many seconds
}
```

These delays are **suggestive**, not absolute. The test chain may override them
to satisfy EVM constraints. For example, if the delay says "advance 5 blocks"
but the gas limit would be exceeded, the chain creates a new block instead.

### How Delays Are Generated

The sequence generator picks delays randomly, capped by config:

```go
blockNumberDelay = random_uint64() % (MaxBlockNumberDelay + 1)
blockTimestampDelay = random_uint64() % (MaxBlockTimestampDelay + 1)
```

Then it enforces a critical invariant: **each block must have a unique
timestamp**. So the block number delay is capped to the timestamp delay:

```go
if blockNumberDelay > blockTimestampDelay {
    if blockTimestampDelay == 0 {
        blockNumberDelay = 0
    } else {
        blockNumberDelay %= blockTimestampDelay
    }
}
```

This means:

- `delay = (0, 0)`: add to the current pending block (same block as previous
  call).
- `delay = (1, 1)`: create a new block, 1 block ahead, 1 second ahead.
- `delay = (3, 5)`: create a new block, 3 blocks ahead, 5 seconds ahead.
- `delay = (5, 3)`: capped to `(3, 3)` — cannot jump more blocks than seconds.

## Medusa's Pending Block: The Core Abstraction

Medusa's `TestChain` (in `chain/test_chain.go`) maintains a **pending block**
that sits between the current chain head and the next committed block. It is a
real `types.Block` with a live `vm.BlockContext`, a `state.StateDB`, and a list
of `Messages` (transactions) that have been executed but not yet committed to
the chain.

```go
type TestChain struct {
    // ...
    pendingBlock            *types.Block
    pendingBlockContext     *vm.BlockContext
    pendingBlockChainConfig *params.ChainConfig
    // ...
}
```

### Lifecycle

A pending block goes through three states:

1. **Created** — `PendingBlockCreate()` builds an empty block with a header
   whose `Number` and `Time` are head+1 by default. The block is not yet part of
   the chain.
2. **Transactions added** — `PendingBlockAddTx()` executes a `core.Message`
   against the pending block's EVM context, updates `GasUsed` and `Bloom` in the
   header, and appends the message and its `MessageResult` to the block.
3. **Committed or discarded** —
    - `PendingBlockCommit()` finalizes the block (computes the state root,
      appends it to `chain.blocks`, and reloads state from the DB).
    - `PendingBlockDiscard()` rolls back all pending transactions, reverts
      state, and drops the block (used when a sequence is abandoned mid-flight).

### State Safety: The `HasPendingStateChanges` Gate

Medusa tracks whether any transaction in the pending block performed a
**contract creation** or **self-destruct**:

```go
func (t *TestChain) HasPendingStateChanges() bool {
    for _, result := range t.pendingBlock.MessageResults {
        for _, change := range result.ContractDeploymentChanges {
            if change.SelfDestructed || change.Creation {
                return true
            }
        }
    }
    return false
}
```

If the answer is **yes**, the pending block **cannot** be mutated in-place.
Changing `block.number` or `block.timestamp` after a contract has been created
or destroyed would corrupt the contract's deployed-address calculation (which
depends on the sender nonce and block context). In that case the fast path is
skipped and the block is committed first.

### How the Sequence Executor Uses the Pending Block

`ExecuteCallSequenceIteratively` (in `fuzzing/calls/call_sequence_execution.go`)
decides for every call whether to mutate the existing pending block or commit it
and start a new one:

```go
if chain.PendingBlock() != nil && call.BlockNumberDelay > 0 {
    if !chain.HasPendingStateChanges() && chain.PendingBlockContext() != nil {
        // FAST PATH: mutate pending block header directly (like vm.roll/vm.warp)
        numberDelay := call.BlockNumberDelay
        timeDelay   := call.BlockTimestampDelay
        if timeDelay == 0 { timeDelay = 1 }
        if numberDelay > timeDelay { numberDelay = timeDelay }

        chain.PendingBlockContext().BlockNumber.Add(
            chain.PendingBlockContext().BlockNumber, big.NewInt(int64(numberDelay)))
        chain.PendingBlock().Header.Number.Set(
            chain.PendingBlockContext().BlockNumber)
        chain.PendingBlockContext().Time += timeDelay
        chain.PendingBlock().Header.Time = chain.PendingBlockContext().Time
    } else {
        // SLOW PATH: must commit before we can change block context
        chain.PendingBlockCommit()
    }
}

if chain.PendingBlock() == nil {
    // No pending block yet — create one with the requested delay
    numberDelay := call.BlockNumberDelay
    timeDelay   := call.BlockTimestampDelay
    if numberDelay == 0 { numberDelay = 1 }
    if timeDelay   == 0 { timeDelay   = 1 }
    if numberDelay > timeDelay { numberDelay = timeDelay }

    chain.PendingBlockCreateWithParameters(
        chain.Head().Header.Number.Uint64() + numberDelay,
        chain.Head().Header.Time + timeDelay,
        nil,
    )
}

// Now add the transaction to the (possibly mutated) pending block
chain.PendingBlockAddTx(call.ToCoreMessage())
```

### What `BlockNumberDelay == 0` Really Means

When a call has `BlockNumberDelay == 0`, the executor skips the "advance block"
branch entirely. If a pending block already exists, the call is simply appended
to it. This means **multiple calls can share the same block**, exactly as real
Ethereum does.

| Delay                                            | Behaviour                                     |
| ------------------------------------------------ | --------------------------------------------- |
| `(0, 0)`                                         | Append to existing pending block (same block) |
| `(1, 1)`                                         | Advance pending block by 1 (fast path)        |
| `(5, 5)`                                         | Advance pending block by 5 (fast path)        |
| `(5, 5)` with contract creation in pending block | Commit first, then create new block at head+5 |

### Reverting and Cloning

Because the pending block mutates live state, Medusa needs clean checkpoints for
shrinking and re-execution:

- **Revert** — `RevertToBlockIndex(n)` discards all blocks after index `n`,
  drops any pending block, reloads state from the trie DB, and fires
  `OnRevertHookFuncs` for every removed transaction.
- **Clone** — `Clone()` reconstructs a brand-new chain from genesis, replays
  every committed block through `PendingBlockCreate`/`AddTx`/`Commit`, and
  verifies that the new head hash matches the original. The pending block is
  **not** cloned; callers must rebuild it if needed.

Both operations rely on the trie-backed `state.StateDB` so that reverted or
cloned chains see exactly the same committed state as the original.

## Recording the Actual Execution Context

After a transaction is added to a block, Medusa captures an **immutable
snapshot** of the execution context:

```go
callSequenceElement.ChainReference = &CallSequenceElementChainReference{
    Block:            pendingBlock,
    TransactionIndex: len(pendingBlock.Messages) - 1,
    BlockNumber:      pendingBlock.Header.Number.Uint64(),   // snapshot
    BlockTimestamp:   pendingBlock.Header.Time,              // snapshot
}
```

These snapshots are what the output formatter displays:

```
1) SimpleKnob.setKnob(42) (block=1, time=1, gas=3000000, gasprice=1, value=0, sender=0x...)
2) SimpleKnob.property_caught() (block=5, time=10, gas=3000000, gasprice=1, value=0, sender=0x...)
```

Notice the gap: `setKnob` executed at block `1`, but `property_caught` at block
`5` because the sequence generator chose a `BlockNumberDelay = 4`. The property
test was executed in a different block context.

## Raptor's Implementation

Raptor now models explicit block delays, matching Medusa's approach:

1. **Per-call delay fields** — `Call` carries `block_number_delay` and
   `block_timestamp_delay`:

    ```rust
    pub struct Call {
        pub selector: [u8; 4],
        pub args: Vec<u8>,
        pub block_number_delay: u64,
        pub block_timestamp_delay: u64,
    }
    ```

2. **Delay capping invariant** — `Call::cap_delays()` enforces Medusa's rule
   that `block_number_delay <= block_timestamp_delay`:

    ```rust
    pub fn cap_delays(&mut self) {
        if self.block_number_delay > self.block_timestamp_delay {
            if self.block_timestamp_delay == 0 {
                self.block_number_delay = 0;
            } else {
                self.block_number_delay %= self.block_timestamp_delay;
            }
        }
    }
    ```

3. **Configuration caps** — `FuzzConfig` exposes CLI flags for max delays:

    ```rust
    #[arg(long = "max-block-delay", default_value = "5")]
    pub max_block_number_delay: u64,

    #[arg(long = "max-time-delay", default_value = "5")]
    pub max_block_timestamp_delay: u64,
    ```

4. **Delay generation during mutation** — `SequenceDelayMutator` randomly
   assigns delays to each call in a sequence, then caps them:

    ```rust
    call.block_number_delay = state.rand_mut().next() % (self.max_block_delay + 1);
    call.block_timestamp_delay = state.rand_mut().next() % (self.max_time_delay + 1);
    call.cap_delays();
    ```

5. **EVM application** — Before executing a call, Raptor applies the delay to
   the EVM block context:

    ```rust
    evm.ctx.block.number += U256::from(call.block_number_delay);
    evm.ctx.block.timestamp += U256::from(call.block_timestamp_delay);
    ```

6. **Actual execution context recording** — After each call, `CallMeta` stores
   the real block number and timestamp seen by the EVM:

    ```rust
    pub struct CallMeta {
        pub block_number: u64,
        pub block_timestamp: u64,
    }
    ```

7. **Failure output** — `format_failure` displays delays when they are non-zero,
   appended after the sender field:

    ```
    1) SimpleKnob.setKnob(42) (block=1, time=1, gas=3000000, gasprice=1, value=0, sender=0x..., blocknumberdelay=2, blocktimestampdelay=3)
    ```

### What Raptor Is Missing

Raptor does not implement a pending block abstraction. Instead:

- Every call is executed immediately via `evm.transact()`, which mutates the
  `StateDB` right away.
- Block delays are applied by editing `evm.ctx.block.number` and
  `evm.ctx.block.timestamp` directly before the next call.
- There is no `PendingBlockCommit` or `PendingBlockDiscard` step — the state is
  never rolled back mid-sequence.

This means Raptor **cannot**:

1. **Pack multiple calls into the same block** — each call gets its own block
   context because the EVM state is committed immediately. A `delay = (0, 0)`
   still creates a new block in practice, even though the block number does not
   advance.
2. **Roll back an incomplete sequence** — Medusa can discard a pending block and
   revert to the last committed state. Raptor has no such checkpoint; once a
   transaction is executed, the state change is permanent for that run.
3. **Shrink by removing a middle call and replaying from a clean state** —
   Medusa clones the chain, replays committed blocks, then rebuilds the
   shortened sequence on a fresh pending block. Raptor would need to rebuild the
   entire EVM from scratch to achieve the same isolation.
4. **Respect `HasPendingStateChanges` safety** — Raptor never checks whether a
   contract creation or self-destruct has occurred before mutating the block
   context. This is safe in practice because Raptor commits every tx, but it
   also loses the expressiveness of multi-tx blocks.

Implementing a true pending block would require:

- Adding a `PendingBlock` struct that wraps a `revm::State` snapshot and a list
  of executed transactions.
- Separating `transact()` (which runs the EVM) from `commit()` (which persists
  state to the DB).
- Tracking contract-creation / self-destruct flags per transaction to guard the
  fast-path `vm.roll`/`vm.warp` mutation.
- Implementing chain clone/revert so that shrinking and corpus replay can start
  from a known committed state.
