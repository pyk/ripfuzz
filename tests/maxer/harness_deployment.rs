//! `ripfuzz max` compiles the harness via solc and deploys it on a sandbox
//! chain, logging the deployed address on success.
//!
//! The fixtures under `fixtures/maxer/harness-deployment` are sources of the
//! project rooted at the current directory, and compilation artifacts are
//! shared under `./.ripfuzz/solc` namespaced by the harness source path. Each
//! test runs against a fresh temp corpus directory.

use std::fs;
use std::path::PathBuf;

use ripfuzz::cli::default_threads;
use ripfuzz::cli::max::{Args, run};

const HARNESS: &str =
    "fixtures/maxer/harness-deployment/HarnessWithIncrement.sol:HarnessWithIncrement";
const REVERTING: &str = "fixtures/maxer/harness-deployment/HarnessWithRevertingConstructor.sol:HarnessWithRevertingConstructor";
const REVERTING_SETUP: &str =
    "fixtures/maxer/harness-deployment/HarnessWithRevertingSetup.sol:HarnessWithRevertingSetup";
const REVERTING_VALUE: &str =
    "fixtures/maxer/harness-deployment/HarnessWithRevertingValue.sol:HarnessWithRevertingValue";
const SETUP: &str = "fixtures/maxer/harness-deployment/HarnessWithSetup.sol:HarnessWithSetup";
const WRONG_NAME: &str = "fixtures/maxer/harness-deployment/HarnessWithIncrement.sol:DoesNotExist";

fn args(harness: &str) -> Args {
    let corpus_dir = std::env::temp_dir().join(format!(
        "ripfuzz-max-deployment-{}-{}",
        std::process::id(),
        fastrand::u64(..)
    ));
    Args {
        harness: harness.parse().unwrap(),
        config: PathBuf::from("./ripfuzz.toml"),
        root: PathBuf::from("."),
        threads: default_threads(),
        max_fuzz_runs: 256,
        max_shrink_runs: 10_000,
        max_calls: 8,
        timeout: None,
        target_value: None,
        corpus_dir,
        quiet: true,
        log_level: tracing::Level::INFO,
    }
}

/// A valid harness must compile and deploy, printing the deployed address.
#[test]
fn max_compiles_and_deploys_harness() {
    run(args(HARNESS)).expect("max should compile and deploy the harness");
}

/// A harness whose constructor reverts must fail deployment with a clear
/// error, and the execution trace must be dumped to the traces directory.
#[test]
fn max_fails_when_harness_constructor_reverts() {
    let err = run(args(REVERTING)).expect_err("max must fail when deployment reverts");
    assert_eq!(
        err.to_string(),
        "harness contract `HarnessWithRevertingConstructor` deployment failed"
    );

    assert!(
        traces_contain(".ripfuzz/traces", "[revert] nope"),
        "a dumped trace must contain the revert reason"
    );
}

/// Whether any dumped trace file under the directory contains the needle.
///
/// Tests run in parallel and share the traces directory, so the most recent
/// trace file may belong to another test.
fn traces_contain(dir: &str, needle: &str) -> bool {
    fs::read_dir(dir)
        .expect("traces dir must exist")
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "log"))
        .any(|entry| {
            fs::read_to_string(entry.path())
                .map(|trace| trace.contains(needle))
                .unwrap_or(false)
        })
}

/// A harness whose `setup` reverts must fail after deployment, with the
/// execution trace dumped to the traces directory.
#[test]
fn max_fails_when_setup_reverts() {
    let err = run(args(REVERTING_SETUP)).expect_err("max must fail when setup reverts");
    assert_eq!(
        err.to_string(),
        "harness contract `HarnessWithRevertingSetup` setup failed"
    );

    assert!(
        traces_contain(".ripfuzz/traces", "[revert] setup failed"),
        "a dumped trace must contain the revert reason"
    );
}

/// A harness whose `value` reverts after setup must fail with the execution
/// trace dumped to the traces directory.
#[test]
fn max_fails_when_value_reverts() {
    let err = run(args(REVERTING_VALUE)).expect_err("max must fail when value reverts");
    assert_eq!(
        err.to_string(),
        "harness contract `HarnessWithRevertingValue` value call failed"
    );

    assert!(
        traces_contain(".ripfuzz/traces", "[revert] value failed"),
        "a dumped trace must contain the revert reason"
    );
}

/// A harness with a working `setup` must compile, deploy, and run `setup`.
#[test]
fn max_runs_setup_after_deployment() {
    run(args(SETUP)).expect("max must run setup after deployment");
}

/// A harness path that exists but names a missing contract must fail with
/// the available contracts listed.
#[test]
fn max_fails_when_contract_name_is_wrong() {
    let err = run(args(WRONG_NAME)).expect_err("max must fail for a wrong contract name");
    assert_eq!(
        err.to_string(),
        "contract `DoesNotExist` not found in `fixtures/maxer/harness-deployment/HarnessWithIncrement.sol`, available contracts: HarnessWithIncrement"
    );
}
