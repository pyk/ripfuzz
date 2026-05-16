# Echidna Corpus

This document describes how Echidna manages its on-disk and in-memory corpus of
transaction sequences used for coverage-guided fuzzing.

---

## 1. Corpus Format

### 1.1 In-Memory Representation

The in-memory corpus type is defined in `lib/Echidna/Types/Corpus.hs`:

```haskell
type Corpus = Set (Int, [Tx])
```

Each element is a pair:

- **`Int`** -- a weight (the call-sequence number at which the entry was
  discovered).
- **`[Tx]`** -- the transaction sequence that produced new coverage.

The corpus is stored as a `Set`, which deduplicates entries automatically (two
sequences with the same weight and identical transactions are considered the
same element). The `Int` weight is used during mutation selection: newer entries
(larger weight) are preferred.

### 1.2 Transaction Type (`Tx`)

A single transaction is defined in `lib/Echidna/Types/Tx.hs`:

```haskell
data Tx = Tx
  { call     :: !TxCall       -- ^ Call payload (CREATE, CALL, raw calldata, or no-call)
  , src      :: !Addr         -- ^ Origin (sender)
  , dst      :: !Addr         -- ^ Destination contract
  , gas      :: !Word64       -- ^ Gas limit
  , gasprice :: !W256         -- ^ Gas price
  , value    :: !W256         -- ^ Value (ETH) transferred
  , delay    :: !(W256, W256) -- ^ (time delay seconds, block delay count)
  }
```

The `TxCall` sum type has four constructors:

| Constructor               | Meaning                                             |
| ------------------------- | --------------------------------------------------- |
| `SolCreate !ByteString`   | Contract creation with init bytecode                |
| `SolCall !SolCall`        | Function call with decoded arguments                |
| `SolCalldata !ByteString` | Raw calldata (used for ABIv2 or unknown signatures) |
| `NoCall`                  | No operation; only advances time/block delay        |

### 1.3 On-Disk File Format

When a `corpusDir` is configured, Echidna persists corpus entries as
**individual JSON files** with a `.txt` extension. The on-disk layout is:

```
<corpusDir>/
  coverage/                  -- sequences that found new coverage
    <hash>.txt
  reproducers/               -- shrunk reproducers for falsified tests
    <hash>.txt
  reproducers-unshrunk/      -- intermediate unshrunk falsified sequences (saved but NOT loaded on startup)
    <hash>.txt
  reproducers-optimizations/ -- intermediate optimization reproducers (saved but NOT loaded on startup)
    <hash>.txt
```

Each `.txt` file contains a **JSON array of `Tx` objects**. Example:

```json
[
    {
        "call": {
            "tag": "SolCall",
            "contents": [
                "transfer",
                [
                    { "tag": "AbiAddress", "contents": "0x1234..." },
                    { "tag": "AbiUInt", "contents": [256, 1000] }
                ]
            ]
        },
        "src": "0x1000000000000000000000000000000000000000",
        "dst": "0x2000000000000000000000000000000000000000",
        "gas": 12500000,
        "gasprice": "0",
        "value": "0",
        "delay": ["0", "0"]
    }
]
```

Notes on serialization:

- `Word256` / `W256` values are serialized as **decimal strings**.
- `ByteString` values are serialized with Haskell `show` (quoted strings).
- `AbiValue` and `AbiType` use Aeson's derived generic JSON encoding.
- `gas` is a JSON number (`Word64`).

#### Legacy Format Compatibility

`parseJSON` for `Tx` supports an older corpus format for backward compatibility
(to be removed in a future major release):

- Field names prefixed with underscore: `_call`, `_src`, `_dst`, `_gas'`,
  `_gasprice'`, `_value`, `_delay`.
- `_gas'` was previously stored as a hex string / `W256`; the parser coerces it
  to `Word64` with `fromIntegral`.

### 1.4 File Naming

Files are named deterministically by hashing the entire transaction sequence:

```haskell
let file = dir </> (show . abs . hash) txSeq <.> "txt"
```

This means:

- Identical sequences always map to the same filename.
- `saveTxs` skips writing if the file already exists
  (`unlessM (doesFileExist file)`), preventing duplicate disk entries.

---

## 2. Corpus Initialization

### 2.1 Empty Corpus at Startup

During environment construction (`mkEnv` in `lib/Echidna.hs`), the shared corpus
reference is initialized empty:

```haskell
corpusRef <- newIORef mempty  -- mempty = Set.empty
```

The `corpusRef :: IORef Corpus` lives inside the global `Env` and is shared
across **all fuzzing workers**.

### 2.2 Loading from Disk

If the user configures `corpusDir` (a `Maybe FilePath` in `CampaignConf`),
Echidna attempts to load existing sequences before starting the campaign:

```haskell
loadInitialCorpus :: Env -> IO [(FilePath, [Tx])]
```

The loader reads **only** two subdirectories:

1. `<corpusDir>/reproducers`
2. `<corpusDir>/coverage`

It **does not** load:

- `reproducers-unshrunk/` -- intermediate unshrunk reproducers (intentionally
  skipped to avoid startup bloat).
- `reproducers-optimizations/` -- intermediate optimization reproducers (also
  skipped).

`loadTxs` behavior:

- Creates the directory if missing.
- Lists all files (ignoring `.` and `..`).
- Reads each file with `decodeStrict` (Aeson JSON parser).
- Silently drops files that fail to parse (`catMaybes`).
- Returns a list of `(FilePath, [Tx])` pairs.
- Prints: `Loaded N transaction sequences from <dir>`.

### 2.3 Distribution to Workers

In `lib/Echidna/UI.hs`, the loaded initial corpus is split into chunks and
distributed across fuzz workers:

```haskell
(corpusChunkSize, largerCorpusChunks) = length initialCorpus `divMod` nFuzzWorkers
corpusChunkSizes =
  replicate largerCorpusChunks (corpusChunkSize + 1) <>
  replicate (nFuzzWorkers - largerCorpusChunks) corpusChunkSize
corpusChunks = splitPlaces corpusChunkSizes initialCorpus ++ repeat []
```

- Each **fuzz worker** receives its own chunk (`corpusChunk`).
- The **symbolic execution worker** receives the **full** `initialCorpus` (not a
  chunk).
- This partitioning reduces contention and keeps workers from replaying the
  exact same startup sequences.

---

## 3. Corpus Loading (Replay)

Before the main fuzzing loop starts, each fuzz worker replays its assigned
initial corpus chunk via `replayCorpus` (`lib/Echidna/Campaign.hs`):

```haskell
replayCorpus :: VM Concrete -> [(FilePath, [Tx])] -> m ()
```

Replay logic:

1. Iterates over each `(file, txSeq)` pair with an index.
2. **Validation:** scans the sequence for any transaction whose destination
   (`dst`) is not present in the current VM's deployed contracts. If found, the
   sequence is skipped and a `TxSequenceReplayFailed` event is emitted.
3. **Execution:** calls `callseq vm txSeq` to execute the full sequence.
4. **Coverage accumulation:** because `callseq` uses `execTxOptC` when coverage
   is enabled, the shared coverage maps are updated. Sequences that still find
   new coverage contribute to the global corpus; sequences that no longer find
   new coverage are naturally omitted from the growing in-memory corpus.
5. Emits `TxSequenceReplayed file i total` for each successfully replayed
   sequence.

This design allows **corpus minimization**: after replay, only sequences that
still improve coverage remain in the shared `corpusRef`.

---

## 4. Corpus Update

### 4.1 Coverage Tracking During Execution

Echidna tracks coverage at the EVM instruction level. When coverage is enabled
(`knownCoverage` is `Just`):

- `execTxOptC` (`lib/Echidna/Campaign.hs`) wraps each transaction execution with
  coverage collection.
- The actual coverage logging happens in `execTxWithCov`
  (`lib/Echidna/Exec.hs`):
    - After every EVM step, the current `(pc, opIx, callDepth)` is recorded in a
      mutable `IOVector` indexed by codehash.
    - A location is considered "new coverage" if the **call stack depth** at
      that PC has not been seen before, or if the **transaction result bit**
      (`Stop`, `Revert`, etc.) at that PC is new.
    - `execTxWithCov` returns `(VMResult Concrete, Bool)` where the `Bool`
      indicates whether coverage grew.

When a transaction finds new coverage, the worker's `newCoverage` flag is set:

```haskell
when grew $ do
  modify' $ \workerState ->
    workerState { newCoverage = True, genDict = ... }
```

### 4.2 Adding Sequences to the In-Memory Corpus

After a full sequence of transactions finishes (`callseq` in
`lib/Echidna/Campaign.hs`):

```haskell
newCoverage <- gets (.newCoverage)
when newCoverage $ do
  ncallseqs <- gets (.ncallseqs)
  newSize <- liftIO $ atomicModifyIORef' env.corpusRef $ \corp ->
    let !corp' = force $ addToCorpus (ncallseqs + 1) results corp
    in (corp', corpusSize corp')
  ...
  pushWorkerEvent NewCoverage { points, numCodehashes, corpusSize = newSize, transactions = fst <$> results }
```

Key details:

- `addToCorpus n res corpus` inserts `(n, rtxs)` where `rtxs = fst <$> res` (the
  list of transactions, discarding their `VMResult`).
- **Reverted transactions are not filtered out here** -- the entire sequence is
  stored, even if some intermediate calls reverted. The filtering for "useful"
  transactions happens elsewhere (e.g., `isUselessNoCall`).
- `force` is applied to reduce memory usage by avoiding thunks inside the `Set`.
- `atomicModifyIORef'` guarantees thread-safe updates across all workers.
- The weight `n = ncallseqs + 1` means **later discoveries get higher weights**.

### 4.3 Persisting to Disk

A background listener (`saveCorpusEvent` in `lib/Echidna/Output/Corpus.hs`)
subscribes to the global event queue and writes corpus entries asynchronously.

Events that trigger disk writes:

| Event                          | Subdirectory                 | Content           |
| ------------------------------ | ---------------------------- | ----------------- |
| `TestFalsified test`           | `reproducers-unshrunk/`      | `test.reproducer` |
| `TestOptimized test`           | `reproducers-optimizations/` | `test.reproducer` |
| `NewCoverage { transactions }` | `coverage/`                  | `transactions`    |

`saveCorpusEvent` behavior:

- Only writes if `corpusDir` is configured.
- Extracts the `(subdir, txs)` pair from the event.
- Calls `saveTxs env (dir </> subdir) [txs]`.
- `saveTxs` creates the subdirectory if missing, hashes the sequence for the
  filename, and writes JSON via `encodeFile`.
- If the file already exists, it is skipped (no overwrite).
- IO exceptions are caught and reported as `Failure` campaign events.
- A `ReproducerSaved file` event is emitted whenever a new file is written.

Important notes:

- `reproducers-unshrunk/` and `reproducers-optimizations/` are **written but
  never read** during initialization. This is intentional: there can be many
  intermediate reproducers, and loading them all would slow startup. The final
  shrunk reproducers should eventually be placed in `reproducers/` by the user
  or by future tooling.
- The `coverage/` directory is both loaded and saved, forming the persistent
  seed corpus.

### 4.4 Selection and Mutation

When generating a new sequence (`randseq` in `lib/Echidna/Campaign.hs`), Echidna
uses the current in-memory corpus as a mutation source:

1. Generate `seqLen` fresh random transactions (`randTxs`).
2. Choose a `CorpusMutation` weighted by constants:
    - `RandomAppend` / `RandomPrepend` with a `TxsMutation` (Identity,
      Shrinking, Mutation, Expansion, Swapping, Deletion).
    - `RandomSplice` -- combine two corpus sequences at a random point.
    - `RandomInterleave` -- interleave two corpus sequences.
3. If the corpus is empty, fall back to `randTxs`.

Corpus selection (`selectFromCorpus` in `lib/Echidna/Mutator/Corpus.hs`):

```haskell
selectFromCorpus =
  weighted . map (\(i, txs) -> (txs, fromIntegral i)) . Set.toDescList
```

- Entries are listed in **descending weight order**.
- The selection probability is proportional to the weight `i`.
- Because newer entries have higher weights, the fuzzer naturally biases toward
  recently discovered coverage.

### 4.5 Event Propagation and UI Visibility

Whenever the corpus grows, a `NewCoverage` worker event is emitted containing:

- `points` -- number of unique instruction PCs covered.
- `numCodehashes` -- number of distinct contracts hit.
- `corpusSize` -- current size of the in-memory corpus.
- `transactions` -- the newly added sequence.

The UI (interactive and non-interactive) listens to these events and prints
messages like:

```
New coverage: 42 instr, 3 contracts, 15 seqs in corpus
```

The global `corpusRef` is also read at the end of the campaign for reporting
(`ppCorpus` in `lib/Echidna/UI/Report.hs`).

---

## Summary Table

| Aspect               | Detail                                                                              |
| -------------------- | ----------------------------------------------------------------------------------- |
| **In-memory type**   | `Set (Int, [Tx])`                                                                   |
| **On-disk format**   | JSON array of `Tx` objects per `.txt` file                                          |
| **File naming**      | `abs(hash(txSeq)).txt`                                                              |
| **Config key**       | `corpusDir` in `CampaignConf`                                                       |
| **Load dirs**        | `reproducers/`, `coverage/`                                                         |
| **Save dirs**        | `coverage/`, `reproducers-unshrunk/`, `reproducers-optimizations/`                  |
| **Shared state**     | `IORef Corpus` inside `Env`                                                         |
| **Thread safety**    | `atomicModifyIORef'`                                                                |
| **Weight meaning**   | Call-sequence number (`ncallseqs + 1`); higher = newer = more likely to be selected |
| **Coverage trigger** | New PC+depth or new `TxResult` bit in per-codehash mutable vectors                  |
| **Replay**           | `replayCorpus` validates `dst` contracts before executing each sequence             |
| **Dedup**            | `Set` dedup in memory; `doesFileExist` skip on disk                                 |
