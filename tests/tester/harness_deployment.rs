//! `ripfuzz test` compiles the harness via solc and deploys it on a sandbox
//! chain, fuzzes for broken invariants, shrinks every sequence, and reports.
//!
//! The fixtures under `fixtures/tester/harness-deployment` are sources of the
//! project rooted at the current directory, and compilation artifacts are
//! shared under `./.ripfuzz/solc` namespaced by the harness source path. Each
//! test runs against a fresh temp corpus directory.

use std::fs;
use std::path::PathBuf;

use ripfuzz::cli::default_threads;
use ripfuzz::cli::test::{Args, run};

const HARNESS: &str =
    "fixtures/tester/harness-deployment/HarnessWithIncrement.sol:HarnessWithIncrement";
const FAILING_INVARIANT: &str = "fixtures/tester/harness-deployment/HarnessWithFailingInvariant.sol:HarnessWithFailingInvariant";
const FAILING_HANDLER: &str =
    "fixtures/tester/harness-deployment/HarnessWithFailingHandler.sol:HarnessWithFailingHandler";
const REVERTING: &str = "fixtures/tester/harness-deployment/HarnessWithRevertingConstructor.sol:HarnessWithRevertingConstructor";
const REVERTING_SETUP: &str =
    "fixtures/tester/harness-deployment/HarnessWithRevertingSetup.sol:HarnessWithRevertingSetup";
const SUMMARY: &str =
    "fixtures/tester/harness-deployment/HarnessWithSummary.sol:HarnessWithSummary";
const WRONG_NAME: &str = "fixtures/tester/harness-deployment/HarnessWithIncrement.sol:DoesNotExist";

fn args(harness: &str) -> Args {
    let corpus_dir = std::env::temp_dir().join(format!(
        "ripfuzz-test-deployment-{}-{}",
        std::process::id(),
        fastrand::u64(..)
    ));
    Args {
        harness: harness.parse().unwrap(),
        config: PathBuf::from("./ripfuzz.toml"),
        root: PathBuf::from("."),
        threads: default_threads(),
        max_runs: 256,
        max_calls: 8,
        timeout: None,
        max_failures: 8,
        corpus_dir,
        quiet: true,
        log_level: tracing::Level::INFO,
    }
}

/// A valid harness must compile, deploy, and fuzz without broken invariants.
#[test]
fn test_compiles_and_deploys_harness() {
    let broken_invariants = run(args(HARNESS)).expect("test should compile and deploy the harness");
    assert!(broken_invariants.is_empty(), "no invariant should break");
}

/// An `invariant_*` function that bails for reachable state must produce a
/// broken invariant whose bail-emitting call and metadata are captured,
/// deduplicated to one broken invariant per id.
#[test]
fn test_finds_and_shrinks_failing_invariant() {
    let broken_invariants =
        run(args(FAILING_INVARIANT)).expect("test should complete the campaign");

    assert_eq!(
        broken_invariants.len(),
        1,
        "dedup must collapse identical ids"
    );
    let broken = &broken_invariants[0];
    let last = broken.sequence().calls().last().unwrap();
    assert_eq!(last.signature(), "invariant_total_below_limit()");
    assert_eq!(broken.id(), "INV-001");
    assert_eq!(broken.description(), "total exceeded 100");
    assert_eq!(broken.reason_display(), "total exceeded 100");
    assert!(
        broken.sequence().len() <= 2,
        "the shrunk sequence must be at most one handler call plus the invariant check"
    );
}

/// A handler that bails for reachable arguments must produce a broken
/// invariant whose bail-emitting call is the handler.
#[test]
fn test_finds_failing_handler() {
    let broken_invariants = run(args(FAILING_HANDLER)).expect("test should complete the campaign");

    assert!(
        !broken_invariants.is_empty(),
        "the failing handler must be found"
    );
    let broken = &broken_invariants[0];
    let last = broken.sequence().calls().last().unwrap();
    assert_eq!(last.signature(), "deposit(uint256)");
    assert_eq!(broken.id(), "HAN-001");
    assert_eq!(broken.reason_display(), "total exceeded 1000");
}

/// A harness whose constructor reverts must fail deployment with a clear
/// error, and the execution trace must be dumped to the traces directory.
#[test]
fn test_fails_when_harness_constructor_reverts() {
    let err = run(args(REVERTING)).expect_err("test must fail when deployment reverts");
    assert_eq!(
        err.to_string(),
        "harness contract `HarnessWithRevertingConstructor` deployment failed"
    );

    assert!(
        traces_contain(".ripfuzz/traces", "[revert] nope"),
        "a dumped trace must contain the revert reason"
    );
}

/// A harness whose `setup` reverts must fail after deployment, with the
/// execution trace dumped to the traces directory.
#[test]
fn test_fails_when_setup_reverts() {
    let err = run(args(REVERTING_SETUP)).expect_err("test must fail when setup reverts");
    assert_eq!(
        err.to_string(),
        "harness contract `HarnessWithRevertingSetup` setup failed"
    );

    assert!(
        traces_contain(".ripfuzz/traces", "[revert] setup failed"),
        "a dumped trace must contain the revert reason"
    );
}

/// A harness with a working `summary` must run it even when no assertion
/// fails, saving an execution trace of the summary call.
#[test]
fn test_runs_summary_without_broken_invariants() {
    run(args(SUMMARY)).expect("test must run the summary after the campaign");

    assert!(
        traces_exist(".ripfuzz/traces"),
        "the summary call must save an execution trace"
    );
}

/// A harness path that exists but names a missing contract must fail with
/// the available contracts listed.
#[test]
fn test_fails_when_contract_name_is_wrong() {
    let err = run(args(WRONG_NAME)).expect_err("test must fail for a wrong contract name");
    assert_eq!(
        err.to_string(),
        "contract `DoesNotExist` not found in `fixtures/tester/harness-deployment/HarnessWithIncrement.sol`, available contracts: HarnessWithIncrement"
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

/// Whether any dumped trace file exists under the directory.
fn traces_exist(dir: &str) -> bool {
    fs::read_dir(dir)
        .expect("traces dir must exist")
        .filter_map(|entry| entry.ok())
        .any(|entry| entry.path().extension().is_some_and(|ext| ext == "log"))
}
