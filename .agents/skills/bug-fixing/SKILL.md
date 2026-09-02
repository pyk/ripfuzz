---
name: bug-fixing
description: Apply bug fixing when a user-provided bug report describes
  ripfuzz behavior that contradicts the expected behavior. Use when a ripfuzz
  bug needs a source fix.
---

# Ripfuzz Fix Bugs Skill

You fix ripfuzz bugs from user-provided bug reports. A bug is any behavior that
contradicts the expected behavior described in the bug report, such as a wrong
shrinker result, an incorrect campaign outcome, or an invalid harness error.

-------------------------------------------------------------------------------

## Input

| #   | Name         | Path                                            |
| --: | :----------- | :---------------------------------------------- |
|   1 | Bug report   | Provided by the user (no file)                  |
|   2 | Harness path | `path/Harness.sol:Contract` from the bug report |

Ripfuzz compiles Solidity directly with `solc` and does not depend on Foundry.
A harness is a plain `.sol` file referenced as `path/Harness.sol:Contract`.

-------------------------------------------------------------------------------

## Output

| #   | Name            | Path                                                                                                                   |
| --: | :-------------- | :--------------------------------------------------------------------------------------------------------------------- |
|   1 | Regression test | Test function in `tests/` or the affected source module's `#[cfg(test)] mod tests` (skip for simple fixes, see FIX-26) |
|   2 | Test fixture    | Solidity source under `fixtures/<area>/<feature>/` (skip for simple fixes, see FIX-26)                                 |
|   3 | Fixed source    | Affected source under `src/`                                                                                           |
|   4 | Changelog entry | `CHANGELOG.md` under `[Unreleased]`                                                                                    |

-------------------------------------------------------------------------------

## Rules

| ID     | Rule                                                                                                                                         |
| :----- | :------------------------------------------------------------------------------------------------------------------------------------------- |
| FIX-01 | You MUST reproduce the bug with the exact command or scenario from the bug report before diagnosing                                          |
| FIX-02 | When the bug report gives a CLI command, the reproduction MUST use `cargo run --`                                                            |
| FIX-03 | Expected behavior MUST be derived from the bug report, `docs/`, and the fixture's Solidity source and solc output                            |
| FIX-04 | You MUST NOT modify the reported harness or its imports before reproducing the bug                                                           |
| FIX-05 | You MUST identify the root cause before creating the regression test                                                                         |
| FIX-06 | When diagnosing, you MAY add permanent `tracing::debug!` statements in the affected source module                                            |
| FIX-07 | When the failing command supports `--log-level`, you MUST run it with `--log-level debug` to reveal the `tracing::debug!` output             |
| FIX-08 | When the failing command lacks log-level control, you MUST add `--log-level debug` support to it as part of the bug fix instead of deferring |
| FIX-09 | You MUST create the regression test before fixing the bug unless FIX-26 applies                                                              |
| FIX-10 | The regression fixture MUST include a Solidity source file under `fixtures/<area>/<feature>/`                                                |
| FIX-11 | The regression test MUST assert the exact expected behavior with `assert_eq!`                                                                |
| FIX-12 | Tests MUST compile fixtures with the `Solc` builder into a temporary out directory, artifacts are generated at test time                     |
| FIX-13 | Fixture artifacts MUST NOT be created, committed, or edited manually                                                                         |
| FIX-14 | The regression test MUST fail against the unfixed code                                                                                       |
| FIX-15 | The regression test failure MUST reproduce the reported bug                                                                                  |
| FIX-16 | You MUST fix the bug only after the regression test reproduces it unless FIX-26 applies                                                      |
| FIX-17 | After the fix, the regression test MUST pass                                                                                                 |
| FIX-18 | When the feature has a golden output file, the regression test MUST assert the full output with `assert_eq!` against it                      |
| FIX-19 | The regression test MUST NOT use `.contains()` for its assertions                                                                            |
| FIX-20 | Solc artifact JSON MUST be explored with `python3 -c` one-liners                                                                             |
| FIX-21 | When the fixture source or expected output already exists, you MUST abort before creating or overwriting it                                  |
| FIX-22 | When consulting crate documentation, you MUST use `cargo txt`                                                                                |
| FIX-23 | You MUST run `make lint` before finishing                                                                                                    |
| FIX-24 | You MUST run `make test` before finishing                                                                                                    |
| FIX-25 | You MUST add a `### Fixed` entry for the bug to `CHANGELOG.md` under `[Unreleased]` before finishing                                         |
| FIX-26 | Simple fixes with no observable behavior change (for example log-level or message rewording) MUST NOT create a regression test or fixture    |

-------------------------------------------------------------------------------

## Workflow

1. Reproduce the bug.
   - Run the exact command from the bug report entry, or write the minimal
     scenario from the report when it gives no command. The subcommand depends
     on the reported feature (`test`, `max`, or `exec`):

     ```bash
     cargo run -- test <HARNESS> --log-level debug
     ```

     `<HARNESS>` is `path/Harness.sol:Contract` relative to `--root` (default
     `.`). Compilation is driven by `ripfuzz.toml` (`[solc]` section).

   - Compare the reproduced output with the report's actual behavior.

   - Verify the report's expected behavior against the fixture's Solidity
     source and solc output, and `docs/`.

   - Stop when the command no longer reproduces the reported bug.

2. Find the root cause.
   - Explore the solc artifact JSON under the solc out dir (default
     `.ripfuzz/solc/`, namespaced by the harness source path) with `python3 -c`
     one-liners:

     ```bash
     python3 -c "import json; a = json.load(open('.ripfuzz/solc/<path>/<File>.sol/<Contract>.json')); print(a['ast']['nodes'][0]['name'])"
     ```

   - Add permanent `tracing::debug!` statements in the affected source module
     when the resolution path is unclear.

   - Run the failing command with `--log-level debug` to reveal the debug
     output:

     ```bash
     cargo run -- test <HARNESS> --log-level debug
     ```

   - Trace the affected feature's resolution path until the root cause explains
     the wrong behavior.

3. Create the regression test before fixing the bug (skip for simple fixes, see
   FIX-26).
   - Determine the affected feature from the bug report, for example
     `--max-failures` or the shrinker.

   - Abort when `fixtures/<area>/<feature>/` already contains the fixture.

   - Add the fixture as plain Solidity source under
     `fixtures/<area>/<feature>/`. No build step and no build config: tests
     compile the source with the `Solc` builder into a temporary out directory
     at test time.

   - Add a test in the affected feature's test module under `tests/` or the
     affected source module's `#[cfg(test)] mod tests`, matching the module's
     existing test helpers, and assert the exact expected behavior with
     `assert_eq!`.

4. Confirm the regression test fails.
   - Run the new test:

     ```bash
     cargo test <test_name>
     ```

   - Verify the failure matches the reported wrong behavior.

   - Verify the failure is not caused by a fixture or build error.

5. Fix the bug.
   - Fix the root cause in the affected source module.

   - When the failing command lacks log-level control, add `--log-level debug`
     support to it instead of deferring.

6. Verify the fix.
   - Run the regression test and confirm it passes (skip when FIX-26 applies):

     ```bash
     cargo test <test_name>
     ```

   - Add a `### Fixed` entry for the bug to `CHANGELOG.md` under `[Unreleased]`
     before finishing.

   - Run `make lint` and `make test` before finishing:

     ```bash
     make lint
     make test
     ```

-------------------------------------------------------------------------------

## Test Template

Match the affected feature's existing test module under `tests/` or the
`#[cfg(test)] mod tests` block in the affected source module. Fixtures live
under `fixtures/` grouped by area, then feature:

| Area        | Fixture directory              | Tests              |
| :---------- | :----------------------------- | :----------------- |
| `tester`    | `fixtures/tester/<feature>/`   | `tests/tester/`    |
| `maxer`     | `fixtures/maxer/<feature>/`    | `tests/maxer/`     |
| `evm`       | `fixtures/evm/<feature>/`      | `tests/evm/`       |
| `executor`  | `fixtures/executor/<feature>/` | `tests/executor/`  |
| `compilers` | `fixtures/compilers/`          | `tests/compilers/` |

Copy an existing test from the affected module and adapt it to the new fixture.
Compile the fixture with the `Solc` builder into a temporary out directory:

```rust
const VERSION: &str = "0.8.36";
const DIR: &str = "fixtures/tester/<feature>";

fn solc_output(name: &str) -> SolcOutput {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("out");
    Solc::new()
        .with_version(VERSION)
        .with_target(format!("{DIR}/{name}.sol"))
        .with_out(&out)
        .compile()
        .unwrap_or_else(|err| panic!("fixture {name} must compile: {err}"))
}
```

Name the test function after the bug. When the feature has golden output, store
it under the fixture (for example `expected/` for trace output, `reports/` for
coverage) and assert with `assert_eq!` against the file contents.
