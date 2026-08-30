//! `ripfuzz max` compiles the harness via solc and deploys it on a sandbox
//! chain, printing the deployed address on success.
//!
//! The fixtures under `fixtures/max-harness-deployment` are sources of the
//! project rooted at the current directory, and compilation artifacts are
//! shared under `./.ripfuzz/out` namespaced by the harness source path.

use std::fs;
use std::path::PathBuf;

use ripfuzz::cli::max::{Args, run};

const HARNESS: &str =
    "fixtures/max-harness-deployment/HarnessWithIncrement.sol:HarnessWithIncrement";
const REVERTING: &str = "fixtures/max-harness-deployment/HarnessWithRevertingConstructor.sol:HarnessWithRevertingConstructor";
const REVERTING_SETUP: &str =
    "fixtures/max-harness-deployment/HarnessWithRevertingSetup.sol:HarnessWithRevertingSetup";
const WRONG_NAME: &str = "fixtures/max-harness-deployment/HarnessWithIncrement.sol:DoesNotExist";

fn args(harness: &str) -> Args {
    Args {
        harness: harness.parse().unwrap(),
        config: PathBuf::from("./ripfuzz.toml"),
        root: None,
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

    let trace_file = latest_trace_file(".ripfuzz/traces");
    let trace = fs::read_to_string(&trace_file).expect("execution trace file must exist");
    assert!(
        trace.contains("[revert] nope"),
        "trace must contain the revert reason:\n{trace}"
    );
}

/// The most recently written `.log` file under the given traces directory.
fn latest_trace_file(dir: &str) -> PathBuf {
    fs::read_dir(dir)
        .expect("traces dir must exist")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "log"))
        .max_by_key(|path| fs::metadata(path).and_then(|meta| meta.modified()).ok())
        .expect("trace file must exist")
}

/// A harness whose `setup` reverts must still deploy: `setup` runs after
/// deployment, so a reverting `setup` must not affect the deploy step.
#[test]
fn max_deploys_harness_with_reverting_setup() {
    run(args(REVERTING_SETUP)).expect("max must deploy a harness with a reverting setup");
}

/// A harness path that exists but names a missing contract must fail with
/// the available contracts listed.
#[test]
fn max_fails_when_contract_name_is_wrong() {
    let err = run(args(WRONG_NAME)).expect_err("max must fail for a wrong contract name");
    assert_eq!(
        err.to_string(),
        "contract `DoesNotExist` not found in `fixtures/max-harness-deployment/HarnessWithIncrement.sol`, available contracts: HarnessWithIncrement"
    );
}
