//! `--stop-on-revert` halts a `ripfuzz test` campaign on the first matching
//! revert and records it as a `REVERT:` finding, while plain campaigns ignore
//! non-invariant reverts.

use std::path::PathBuf;

use alloy_sol_types::SolError as _;
use ripfuzz::cli::test::Command;
use ripfuzz::tester::StopOnRevert;

alloy_sol_types::sol! {
    error CustomRevert();
}

const HARNESS: &str =
    "fixtures/tester/stop-on-revert/HarnessWithCustomRevert.sol:HarnessWithCustomRevert";

fn command(stop_on_revert: Option<StopOnRevert>) -> Command {
    let corpus_dir = std::env::temp_dir().join(format!(
        "ripfuzz-test-stop-on-revert-{}-{}",
        std::process::id(),
        fastrand::u64(..)
    ));
    Command {
        harness: HARNESS.parse().unwrap(),
        config: PathBuf::from("./ripfuzz.toml"),
        root: PathBuf::from("."),
        threads: 1,
        max_fuzz_runs: 64,
        max_shrink_runs: 64,
        max_calls: 4,
        timeout: None,
        max_failures: 8,
        stop_on_revert,
        stop_on_panic: None,
        corpus_dir,
        quiet: true,
        log_level: tracing::Level::INFO,
    }
}

/// Plain campaigns ignore handler reverts that are not broken invariants.
#[test]
fn without_stop_on_revert_custom_reverts_are_ignored() {
    let broken = command(None).run().expect("campaign must complete");
    assert!(broken.is_empty(), "no broken invariant expected");
}

/// Any revert stops the campaign and records the revert selector.
#[test]
fn stop_on_any_revert_records_the_custom_error() {
    let broken = command(Some(StopOnRevert::Any))
        .run()
        .expect("campaign must complete");
    assert_eq!(broken.len(), 1, "exactly one revert finding expected");
    let expected = format!("REVERT:0x{}", hex::encode(CustomRevert::SELECTOR));
    assert_eq!(broken[0].id(), expected);
}

/// A matching selector stops the campaign.
#[test]
fn stop_on_matching_selector_records_the_revert() {
    let selector = format!("0x{}", hex::encode(CustomRevert::SELECTOR));
    let filter: StopOnRevert = selector.parse().unwrap();
    let broken = command(Some(filter)).run().expect("campaign must complete");
    assert_eq!(broken.len(), 1, "exactly one revert finding expected");
    assert_eq!(broken[0].id(), format!("REVERT:{selector}"));
}

/// A non-matching selector never stops the campaign.
#[test]
fn stop_on_other_selector_ignores_the_revert() {
    let filter: StopOnRevert = "0xdeadbeef".parse().unwrap();
    let broken = command(Some(filter)).run().expect("campaign must complete");
    assert!(
        broken.is_empty(),
        "no finding expected for another selector"
    );
}
