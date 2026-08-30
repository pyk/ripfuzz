//! `ripfuzz max` compiles the harness via solc and deploys it on a sandbox
//! chain, printing the deployed address on success.
//!
//! The fixtures under `fixtures/max-harness-deployment` are sources of the
//! project rooted at the current directory, and compilation artifacts are
//! shared under `./.ripfuzz/out` namespaced by the harness source path.

use std::path::PathBuf;

use ripfuzz::cli::max::{Args, run};

const HARNESS: &str =
    "fixtures/max-harness-deployment/HarnessWithIncrement.sol:HarnessWithIncrement";
const REVERTING: &str = "fixtures/max-harness-deployment/HarnessWithRevertingConstructor.sol:HarnessWithRevertingConstructor";

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
/// error.
#[test]
fn max_fails_when_harness_constructor_reverts() {
    let err = run(args(REVERTING)).expect_err("max must fail when deployment reverts");
    assert_eq!(
        err.to_string(),
        "harness contract `HarnessWithRevertingConstructor` deployment failed"
    );
}
