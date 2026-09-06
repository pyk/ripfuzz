//! `--stop-on-panic` halts a `ripfuzz test` campaign on the first matching
//! Solidity panic and records it as a `PANIC:` finding, while plain campaigns
//! ignore panics.

use std::path::PathBuf;

use ripfuzz::cli::test::Command;
use ripfuzz::tester::StopOnPanic;

const HARNESS: &str = "fixtures/tester/stop-on-revert/HarnessWithAssert.sol:HarnessWithAssert";

fn command(stop_on_panic: Option<StopOnPanic>) -> Command {
    let corpus_dir = std::env::temp_dir().join(format!(
        "ripfuzz-test-stop-on-panic-{}-{}",
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
        stop_on_revert: None,
        stop_on_panic,
        corpus_dir,
        quiet: true,
        log_level: tracing::Level::INFO,
    }
}

/// Plain campaigns ignore panicking handlers.
#[test]
fn without_stop_on_panic_asserts_are_ignored() {
    let broken = command(None).run().expect("campaign must complete");
    assert!(broken.is_empty(), "no broken invariant expected");
}

/// Any panic stops the campaign and records the assert code.
#[test]
fn stop_on_any_panic_records_the_assert_code() {
    let broken = command(Some(StopOnPanic::Any))
        .run()
        .expect("campaign must complete");
    assert_eq!(broken.len(), 1, "exactly one panic finding expected");
    assert_eq!(broken[0].id(), "PANIC:0x01");
}

/// A matching panic code stops the campaign.
#[test]
fn stop_on_matching_code_records_the_panic() {
    let filter: StopOnPanic = "0x01".parse().unwrap();
    let broken = command(Some(filter)).run().expect("campaign must complete");
    assert_eq!(broken.len(), 1, "exactly one panic finding expected");
    assert_eq!(broken[0].id(), "PANIC:0x01");
}

/// A non-matching panic code never stops the campaign.
#[test]
fn stop_on_other_code_ignores_the_panic() {
    let filter: StopOnPanic = "0x11".parse().unwrap();
    let broken = command(Some(filter)).run().expect("campaign must complete");
    assert!(broken.is_empty(), "no finding expected for another code");
}
