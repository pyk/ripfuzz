# Medusa Corpus

This document describes the structure, lifecycle, and mechanics of Medusa's
fuzzing corpus. The corpus is a persistent archive of call sequences that drive
coverage-guided mutation and preserve test failures across fuzzing campaigns.

## Overview

The `Corpus` type lives in `fuzzing/corpus/corpus.go`. It stores two kinds of
artifacts:

1. **Coverage-increasing call sequences** — sequences that discovered new code
   coverage. These are used as mutation seeds.
2. **Test-result call sequences** — sequences that triggered a test failure
   (property violation, assertion, etc.). These are kept for reproducibility but
   are **not** used for mutation.

Both kinds are serialized to JSON files on disk so they survive across runs.
When Medusa starts, it loads every file, replays each sequence to verify it
still executes, and rebuilds in-memory coverage maps and a weighted random
chooser for mutation targets.

---

## 1. Corpus Format

### Directory Layout

If `fuzzing.corpusDirectory` is set in the project configuration, Medusa expects
the following layout:

```
<corpusDirectory>/
├── call_sequences/
│   ├── 1716123456789012345-uuid-1.json
│   ├── 1716123456789012345-uuid-2.json
│   └── ...
└── test_results/
    ├── 1716123456789012345-uuid-3.json
    └── ...
```

- **`call_sequences/`** — JSON files for sequences that increased coverage.
- **`test_results/`** — JSON files for sequences that caused a test failure.

If `corpusDirectory` is an empty string, the corpus operates entirely in memory
and never touches disk.

#### Legacy Layout Migration

Older versions of Medusa split coverage sequences into two subdirectories:

```
call_sequences/
├── mutable/
└── immutable/
```

`NewCorpus` detects this legacy structure in `migrateLegacyCorpus()` and moves
all `.json` files up into the flat `call_sequences/` directory before reading
anything. The old subdirectories are then deleted.

### File Naming Convention

Every file name is generated from a nanosecond timestamp and a UUID:

```go
fileName := fmt.Sprintf("%v-%v.json", time.Now().UnixNano(), uuid.New().String())
```

This guarantees uniqueness and chronological ordering.

### JSON Schema

Each `.json` file represents a single `CallSequence`, which is an array of
`CallSequenceElement` objects.

#### `CallSequenceElement`

```json
[
    {
        "call": {
            /* CallMessage */
        },
        "blockNumberDelay": 1,
        "blockTimestampDelay": 1
    }
]
```

| Field                 | Type   | Description                                                                                   |
| --------------------- | ------ | --------------------------------------------------------------------------------------------- |
| `call`                | object | The transaction message (see `CallMessage` below).                                            |
| `blockNumberDelay`    | uint64 | How many blocks to advance before including this call. `0` means the current pending block.   |
| `blockTimestampDelay` | uint64 | How many seconds to advance before including this call. Ignored if `blockNumberDelay` is `0`. |

At runtime, fields such as `Contract`, `ChainReference`, and `ExecutionTrace`
are populated in memory but are **not** serialized to JSON (`json:"-"`).

#### `CallMessage`

```json
{
    "from": "0x...",
    "to": "0x...",
    "nonce": 0,
    "value": "0x0",
    "gasLimit": 3000000,
    "gasPrice": "0x1",
    "gasFeeCap": "0x0",
    "gasTipCap": "0x0",
    "data": "0x...",
    "dataAbiValues": {
        /* optional, see below */
    }
}
```

| Field           | Type          | Description                                          |
| --------------- | ------------- | ---------------------------------------------------- |
| `from`          | address (hex) | Sender address.                                      |
| `to`            | address (hex) | Target address. `null` for contract creation.        |
| `nonce`         | uint64        | Sender nonce.                                        |
| `value`         | hexutil.Big   | ETH value transferred.                               |
| `gasLimit`      | uint64        | Gas limit.                                           |
| `gasPrice`      | hexutil.Big   | Gas price.                                           |
| `gasFeeCap`     | hexutil.Big   | EIP-1559 fee cap.                                    |
| `gasTipCap`     | hexutil.Big   | EIP-1559 tip cap.                                    |
| `data`          | hexutil.Bytes | Raw calldata. Omitted if `dataAbiValues` is present. |
| `dataAbiValues` | object        | ABI-encoded call data (preferred for portability).   |

Note: `AccessList`, `SkipNonceChecks`, and `SkipFromEOACheck` exist on the Go
struct but are not persisted in the corpus JSON.

#### `CallMessageDataAbiValues`

When `dataAbiValues` is used instead of raw `data`, the JSON contains a
human-readable method signature and its arguments. This is the preferred format
because it survives ABI changes (e.g., argument reordering) better than raw
bytes.

```json
{
    "methodSignature": "transfer(address,uint256)",
    "inputValues": [
        "0xAb5801a7D398351b8bE11C439e05C5B3259aeC9B",
        "1000000000000000000"
    ]
}
```

| Field             | Type   | Description                                                                                                             |
| ----------------- | ------ | ----------------------------------------------------------------------------------------------------------------------- |
| `methodSignature` | string | Full function prototype (e.g. `transfer(address,uint256)`). Used to recompute the 4-byte selector at load time.         |
| `inputValues`     | array  | JSON-friendly representation of each ABI argument. Booleans, numbers, strings, and nested arrays/structs are supported. |

At deserialization time, `Resolve(contractAbi)` is called to:

1. Hash the `methodSignature` and look up the matching `abi.Method`.
2. Decode `inputValues` into Go types using
   `valuegeneration.DecodeJSONArgumentsFromSlice`.
3. Populate the runtime `Method` and `InputValues` fields.

If resolution fails (e.g., the contract ABI changed), the sequence is marked
invalid and will not be used for mutations.

### Complete Example

```json
[
    {
        "call": {
            "from": "0x1000000000000000000000000000000000000000",
            "to": "0x2000000000000000000000000000000000000000",
            "nonce": 0,
            "value": "0x0",
            "gasLimit": 3000000,
            "gasPrice": "0x1",
            "gasFeeCap": "0x0",
            "gasTipCap": "0x0",
            "dataAbiValues": {
                "methodSignature": "deposit(uint256)",
                "inputValues": ["1234567890"]
            }
        },
        "blockNumberDelay": 1,
        "blockTimestampDelay": 1
    },
    {
        "call": {
            "from": "0x1000000000000000000000000000000000000000",
            "to": "0x2000000000000000000000000000000000000000",
            "nonce": 1,
            "value": "0x0",
            "gasLimit": 3000000,
            "gasPrice": "0x1",
            "gasFeeCap": "0x0",
            "gasTipCap": "0x0",
            "dataAbiValues": {
                "methodSignature": "withdraw(uint256)",
                "inputValues": ["1000"]
            }
        },
        "blockNumberDelay": 0,
        "blockTimestampDelay": 0
    }
]
```

---

## 2. How It Is Initialized

Initialization happens in two phases: **construction** (`NewCorpus`) and
**runtime setup** (`Initialize`).

### Phase 1 — `NewCorpus(corpusDirectory string)`

1. **Allocate structures**: Creates empty `CoverageMaps`, two `corpusDirectory`
   instances (one for call sequences, one for test results), an empty
   `unexecutedCallSequences` slice, and a logger.
2. **Legacy migration**: If `corpusDirectory` is non-empty,
   `migrateLegacyCorpus()` checks for `call_sequences/mutable/` and
   `call_sequences/immutable/`. If they exist, all `.json` files are moved to
   the parent `call_sequences/` directory and the old folders are deleted.
3. **Read existing files**:
    - `callSequenceFiles.readFiles("*.json")` loads all coverage sequences.
    - `testResultSequenceFiles.readFiles("*.json")` loads all test-result
      sequences.

If the directory does not exist or is empty, the corpus starts blank.

### Phase 2 — `Corpus.Initialize(baseTestChain, contractDefinitions)`

This method is called once by the `Fuzzer` after the base test chain has been
set up (contracts deployed, `setUp()` executed). Its purpose is to:

1. **Reset mutable state**:
    - Instantiate a fresh `WeightedRandomChooser[CallSequence]` for mutation
      targets.
    - Clear `unexecutedCallSequences`.
    - Reset the atomic `validCallSequences` counter to `0`.

2. **Seed coverage maps from the post-setup chain**:
    - Clone `baseTestChain`, attaching a `CoverageTracer`.
    - Subscribe to `ContractDeploymentAddedEvent` and
      `ContractDeploymentRemovedEvent` to build a map of `deployedContracts`.
    - Freeze that map into an "initial contracts set" and feed it to the
      coverage tracer so it knows which contracts to track.
    - Iterate over every already-committed block in the cloned chain, extract
      coverage maps from each `MessageResult`, and merge them into
      `c.coverageMaps` via `CoverageMaps.Update()`.
    - Remove the coverage tracer results from the message results afterward to
      save memory.

3. **Enqueue all loaded sequences for replay**:
    - Append every test-result sequence, then every coverage sequence, into
      `unexecutedCallSequences`.
    - This list represents the backlog of corpus items that must be executed
      (without mutation) before the main fuzzing loop begins.

Why this two-phase design? `NewCorpus` only needs a path on disk and can be
called before the EVM is ready. `Initialize` needs the actual deployed chain so
it can:

- measure how much coverage the setup already achieved, and
- schedule every on-disk sequence for validation against the live contract
  state.

---

## 3. How It Is Loaded

Loading is the process of moving corpus items from disk into runtime structures
and making them executable.

### 3.1 Disk → Memory (`corpusDirectory.readFiles`)

Each subdirectory is managed by a `corpusDirectory[T]` generic type
(`corpus_files.go`). The `readFiles` method:

1. Runs `filepath.Glob(filepath.Join(path, pattern))` to discover files.
2. Reads each file with `os.ReadFile`.
3. Unmarshals the bytes into a `CallSequence` using `json.Unmarshal`.
4. Stores the result in a `corpusFile[T]` struct that tracks:
    - `fileName` — base name for later writes.
    - `data` — the unmarshaled `CallSequence`.
    - `writtenToDisk` — `true` because it came from disk.

At this point the sequences are in memory but are not yet ready for execution
because contract and ABI references are unresolved.

### 3.2 Runtime Resolution (`bindCorpusElement`)

During the pre-fuzzing replay phase, each worker calls
`sequenceGenerator.InitializeNextSequence()`, which pulls the next item from
`unexecutedCallSequences`. Before the call can be executed,
`FuzzerWorker.bindCorpusElement` resolves runtime dependencies:

1. **Contract resolution**: Looks up `element.Call.To` in
   `fw.deployedContracts`. If the address is missing, the sequence is invalid.
2. **ABI resolution**: If `element.Call.DataAbiValues` is non-nil, calls
   `Resolve(contractAbi)` to decode the method signature and input values
   against the compiled contract ABI.

If either step fails, the worker logs a debug message, marks the sequence as
disabled, and skips it.

### 3.3 Pre-Fuzzing Replay (`UnexecutedCallSequence`)

The fuzzer workers consume the `unexecutedCallSequences` list through
`Corpus.UnexecutedCallSequence()`:

- Thread-safe pop from the front of the slice.
- The sequence is executed verbatim (no mutation) via
  `ExecuteCallSequenceIteratively`.
- After execution, if the sequence succeeds and produces no shrink requests,
  `MarkCallSequenceForMutation` adds it to the `mutationTargetSequenceChooser`.
- If execution fails (e.g., ABI mismatch or contract no longer exists), the
  sequence is silently discarded and `validCallSequences` is **not**
  incremented.

Only after every `unexecutedCallSequence` has been processed does the fuzzer
consider the corpus "initialized" (`InitializingCorpus() == false`) and begin
normal mutation-based fuzzing.

---

## 4. How It Is Updated

The corpus is updated dynamically during a fuzzing campaign. There are four main
update paths:

### 4.1 Coverage-Driven Addition (`CheckSequenceCoverageAndUpdate`)

After every call in a fuzzing sequence, the worker checks whether the **last
executed call** achieved new coverage:

```go
coverageUpdated, err := checkSequenceCoverageAndUpdate(callSequence, c.coverageMaps)
```

`checkSequenceCoverageAndUpdate` works like this:

1. Obtain the `MessageResults` for the last call from its `ChainReference`.
2. Extract the coverage maps generated by the `CoverageTracer`.
3. Remove the tracer results from the message results to free memory.
4. Merge those maps into the corpus's global `coverageMaps` via
   `CoverageMaps.Update`.
5. Return `true` if any new coverage markers were added.

If `coverageUpdated == true`, the full sequence is added to the corpus:

```go
c.addCallSequence(c.callSequenceFiles, callSequence, true, mutationChooserWeight, flushImmediately)
```

### 4.2 Deduplication (`addCallSequence`)

Before a sequence is accepted, `addCallSequence` ensures it is unique:

1. Compute `sequence.Hash()`, which hashes every element's `BlockNumberDelay`,
   `BlockTimestampDelay`, and call message hash.
2. Compare against every existing file in the target directory.
3. If a duplicate hash exists, the call returns immediately without adding
   anything.

If the sequence is unique:

- A new `corpusFile` is created with `writtenToDisk = false`.
- If `useInMutations` is true and the chooser is initialized, the sequence is
  added to `mutationTargetSequenceChooser` with the provided weight (default
  `1`).
- If `flushImmediately` is true, `Flush()` is called synchronously.

### 4.3 Weighted Random Chooser (`mutationTargetSequenceChooser`)

The chooser (`randomutils.WeightedRandomChooser`) is the source of mutation
seeds.

- **New coverage sequences** are added with weight `1 + sequencesTested` (via
  `getNewCorpusCallSequenceWeight`) so that sequences discovered later are
  proportionally more likely to be picked.
- **Replayed corpus sequences** are added with weight `1` after they execute
  successfully.
- `RandomMutationTargetSequence` clones the chosen sequence before returning it,
  so the original is never mutated in place.

### 4.4 Test-Result Sequences (`AddTestResultCallSequence`)

When a property test fails, the fuzzer calls `AddTestResultCallSequence`. This
uses the same `addCallSequence` machinery but targets `testResultSequenceFiles`
and sets `useInMutations = false`. These sequences are:

- Written to `test_results/*.json`.
- Never added to the `mutationTargetSequenceChooser`.
- Still replayed on startup to verify they still fail.

### 4.5 Flushing to Disk (`Flush`)

`Flush` is called periodically (e.g., after every sequence addition when
`flushImmediately` is true, or on campaign shutdown):

1. Lock the corpus to prevent concurrent modification.
2. Iterate all files in `callSequenceFiles` and `testResultSequenceFiles`.
3. For any file where `writtenToDisk == false`, marshal it to indented JSON and
   write it with `os.WriteFile(..., os.ModePerm)`.
4. Mark the file as `writtenToDisk = true`.

If `storageDirectory == ""`, `Flush` is a no-op.

### 4.6 Pruning (`PruneSequences`)

Over time the corpus can accumulate redundant sequences that collectively cover
the same code. `PruneSequences` removes unnecessary entries:

1. Snapshot the current `mutationTargetSequenceChooser.Choices` and clone every
   sequence.
2. Create a blank temporary `CoverageMaps` (`tmpMap`).
3. Iterate the cloned sequences in a **random order**.
4. Execute each sequence on a fresh chain (reverting to the base block index
   after each one).
5. After execution, run `checkSequenceCoverageAndUpdate(seq, tmpMap)`.
6. If the sequence **does not** add new coverage to `tmpMap`, it is flagged for
   removal.
7. Remove all flagged sequences from the chooser.

The algorithm finds a smaller subset of sequences that still preserves the
current total coverage. `PruneSequences` is invoked:

- Explicitly by the `medusa corpus clean` CLI (indirectly, because `Clean` only
  validates; pruning is a separate background task).
- Periodically by `CorpusPruner` (see below).

### 4.7 Cleaning Invalid Sequences (`CleanInvalidSequences`)

When contracts are refactored or recompiled, old corpus files may contain stale
addresses or ABI signatures. `CleanInvalidSequences` validates every file in
`callSequenceFiles`:

1. Clone the sequence.
2. For each element, resolve the target contract and ABI values against the
   current `deployedContracts` map.
3. Execute the full sequence on a test chain.
4. If resolution fails or execution errors, the filename is added to
   `InvalidSequences` and the file is deleted from disk via
   `removeFileFromDisk`.
5. Revert the chain to the original block index.

The CLI command `medusa corpus clean` creates a `CorpusCleaner` and runs this
logic, reporting how many sequences were valid vs. invalid.

### 4.8 Background Pruning Job (`CorpusPruner`)

If coverage is enabled and `pruneFrequency > 0` (configured in `medusa.json`),
the fuzzer spawns a `CorpusPruner` goroutine:

- `Start` clones the base test chain and attaches a `CoverageTracer`.
- `mainLoop` sleeps for `pruneFrequency` minutes, then calls `pruneCorpus`.
- `pruneCorpus` times the operation, calls `Corpus.PruneSequences`, logs the
  number removed, and updates `totalCorpusPruned`.
- The loop exits when the fuzzer's context is cancelled.

---

## Summary of Key Types

| Type                       | File                               | Role                                                      |
| -------------------------- | ---------------------------------- | --------------------------------------------------------- |
| `Corpus`                   | `corpus.go`                        | Top-level archive manager.                                |
| `corpusDirectory[T]`       | `corpus_files.go`                  | Generic read/write layer for a JSON directory.            |
| `corpusFile[T]`            | `corpus_files.go`                  | In-memory representation of one on-disk file.             |
| `CallSequence`             | `calls/call_sequence.go`           | Ordered list of calls (the unit stored in the corpus).    |
| `CallSequenceElement`      | `calls/call_sequence.go`           | One call with delays and chain reference.                 |
| `CallMessage`              | `calls/call_message.go`            | EVM message with JSON marshaling generated by `gencodec`. |
| `CallMessageDataAbiValues` | `calls/call_message_abi_values.go` | Portable ABI representation used in the corpus.           |
| `CoverageMaps`             | `coverage/coverage_maps.go`        | Thread-safe coverage aggregation.                         |
| `CorpusPruner`             | `corpus_pruner.go`                 | Periodic background pruning task.                         |
| `CorpusCleaner`            | `corpus_cleaner.go`                | CLI-facing wrapper for `CleanInvalidSequences`.           |

---

## Relevant Source Files

- `fuzzing/corpus/corpus.go` — Core corpus logic: initialization, loading,
  updating, pruning, cleaning.
- `fuzzing/corpus/corpus_files.go` — File I/O and JSON serialization.
- `fuzzing/corpus/corpus_pruner.go` — Background pruning goroutine.
- `fuzzing/corpus/corpus_cleaner.go` — CLI helper for cleaning invalid entries.
- `fuzzing/calls/call_sequence.go` — `CallSequence` and `CallSequenceElement`
  definitions.
- `fuzzing/calls/call_message.go` — `CallMessage` definition and cloning.
- `fuzzing/calls/call_message_abi_values.go` — ABI resolution and JSON
  marshaling for portable call data.
- `fuzzing/calls/gen_call_message_json.go` — Auto-generated JSON codec for
  `CallMessage`.
- `cmd/corpus.go` — `medusa corpus clean` command implementation.
