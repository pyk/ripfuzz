# Fork Mode

This document describes how ripfuzz forking works: opting into remote chain
state with `rvm.fork`, multi-fork isolation, and how harness storage is shared
across chains for cross-chain invariants (for example value conservation).

## Overview

Campaigns always start as an **empty sandbox**. There is no CLI flag to select
an RPC URL or block. Call `rvm.fork` from the harness to attach remote state:

```solidity
function setup() external {
    rvm.fork(rvm.getEnv("ETH_RPC_URL"), 21_000_000);
}
```

You can also call `rvm.fork` from handler actions to switch chains
mid-campaign.

### API

```solidity
struct ForkConfig {
    uint32 retries;
    uint64 backoffMs;
    uint64 timeoutMs;
    uint64 rateLimit;
}

function fork(string calldata url, uint256 blockNumber) external;
function fork(string calldata url, uint256 blockNumber, ForkConfig config)
    external;
```

Single-argument defaults (when `ForkConfig` is omitted):

| Setting   | Default |
| :-------- | :------ |
| retries   | 3       |
| backoffMs | 100     |
| timeoutMs | 30_000  |
| rateLimit | 10      |

Campaigns default to 10 RPC batches per second so public-provider quotas are
not blown on the first run. Override per fork with
`vm.fork(..., ForkConfig{rateLimit: N})`. `rateLimit: 0` disables the limit.

Forks are keyed and cached by `(url, block)`. Selecting an existing key reuses
that fork's overlay instead of re-fetching the block header.

## Two kinds of state

This is the core multi-fork model:

| State               | Examples                                                             | Across `rvm.fork` switches |
| :------------------ | :------------------------------------------------------------------- | :------------------------- |
| **Remote**          | On-chain accounts, balances, storage at forked addresses             | **Isolated per fork**      |
| **Local (harness)** | Harness storage, deployer, `rvm.addr` accounts, contracts you CREATE | **Shared across forks**    |

```text
                    +--------------------+
  rvm.fork(eth) --> | Fork: Ethereum     |  remote WETH, bridges, ...
                    | overlay (isolated) |
                    +--------------------+
                              ^
                              | local accounts copied
                              | (harness storage, ghosts, ...)
                              v
                    +--------------------+
  rvm.fork(poly) -> | Fork: Polygon      |  same address, different remote state
                    | overlay (isolated) |
                    +--------------------+
```

### Remote state is isolated per fork

The same address on two chains keeps independent storage and balances. A
mutation on Ethereum (for example `rvm.store` or `rvm.deal` on a bridge) does
not appear when you switch to Polygon, and switching back restores the Ethereum
overlay including your mutations.

This matches real multi-chain deployments: a bridge or token at the same
address on L1 and L2 is not the same storage.

### Harness storage is shared across chains

The harness contract (and other local accounts) is copied onto every active
fork. Storage slots you write on the harness survive `rvm.fork` switches.

Use that for **ghost variables** and cross-chain bookkeeping:

- amount locked / burned on the source chain
- amount minted / unlocked on the destination chain
- running totals for conservation invariants

Remote protocol state stays per-chain; your accounting lives on the harness.

## Value conservation example

A typical bridge campaign records outflow on one fork and inflow on another,
then asserts they match:

```solidity
uint256 public totalOutflow;
uint256 public totalInflow;

function actionLockOnL1(uint256 amount) external {
    rvm.fork(rvm.getEnv("ETH_RPC_URL"), L1_BLOCK);
    // ... interact with L1 bridge ...
    totalOutflow += amount;
}

function actionMintOnL2(uint256 amount) external {
    rvm.fork(rvm.getEnv("POLYGON_RPC_URL"), L2_BLOCK);
    // ... interact with L2 bridge / token ...
    totalInflow += amount;
}

function invariant_conservation() external view {
    assert(totalOutflow == totalInflow);
}
```

In one call sequence the fuzzer may:

1. Fork Ethereum, lock value, bump `totalOutflow`
2. Fork Polygon, mint value, bump `totalInflow`
3. Switch forks again and still see both totals on the harness
4. Run `invariant_conservation`

Remote bridge storage remains isolated; the ghost totals do not reset on
switch.

## What is copied as local

On each fork select or create, ripfuzz copies accounts marked local:

- the default deployer
- the RVM cheatcode address
- addresses registered by CREATE / CREATE2 (including the harness)
- addresses from `rvm.addr`

Those accounts' balances, nonces, code, and storage travel with you across
forks. Everything else is treated as remote and stays on the active fork's
overlay only.

## RPC cache and isolation between runs

From the RPC side, remote state is read-only:

- Account, storage, and block data are fetched on demand and cached in memory
- The cache is written under the campaign / project cache directory
- Local EVM mutations (deploy, setup, sequence execution) are not written back
  to the node

Each fuzz input starts from a **clone of the post-setup chain**. Writes in one
sequence do not leak into another sequence. Multi-fork overlays are part of
that cloned snapshot after setup has run `rvm.fork`.

## Debugging slow fork campaigns

When a campaign looks stuck (runs or coverage frozen), the progress line
includes RPC counters and the current hotspot handler:

```text
progress runs=44 ... rpc_hit=12,482 rpc_miss=63 rpc_wait=12.4s hot=getQuote hot_elapsed=11.8s hot_rpc_miss=48 ...
```

- `rpc_miss` climbing while `runs` is stuck: the fuzzer is waiting on RPC
  (uncached account or storage reads)
- `rpc_hit` and `rpc_miss` both flat: time is in the EVM, not the node
- `rpc_wait` is time spent in RPC batches, including rate-limit sleeps
- `hot` is the handler (or invariant/max function) with the most wall time;
  `hot=-` means every function is still at zero
- Finished logs still print every handler, with `elapsed`, `rpc_hit`,
  `rpc_miss`, and `rpc_wait` on each row

On the first `rvm.fork`, cache load is logged:

```text
loaded fork cache entries=52237 path=/path/to/project/.ripfuzz/cache
```

A small entry count after prefetching means the cache does not cover the slots
the harness actually reads. `--log-level debug` logs each miss as
`rpc cache miss method=eth_getBalance key=...`.

Transient RPC retries log a one-line warning on the terminal:

```text
WARN run{fuzzer_id=15}: transient RPC error; retrying batch retry=1 retries=3 backoff_ms=100 items=18 url=https://eth-mainnet.g.alchemy.com error=RPC error 429
```

The campaign log file still has the full URL, JSON-RPC payload, and error body.

## Hardfork / SpecId

`rvm.fork` applies the EVM hardfork (`SpecId`) derived from the remote chain id
and block timestamp. Prefer pinning recent blocks (Cancun or later) when the
harness is compiled with a modern solc target (Shanghai+ / Prague), so opcodes
such as `PUSH0` remain valid after the fork.

## Related

- Harness writing guide: [harness-contract.md](./harness-contract.md)
- Blog: [rvm.fork instead of
  --rpc-url](https://probablyrevert.com/blog/2026-08-07-vm-fork-instead-of-cli)
