//! The `max` command runs the optional `summary` function on the final
//! campaign state after saving the corpus, and saves its execution trace
//! alongside the log output it emits.

use std::fs;
use std::path::PathBuf;

use alloy_primitives::U256;
use ripfuzz::cli::default_threads;
use ripfuzz::cli::max::{Args, run};

/// The summary call must appear in a saved trace after the campaign.
#[test]
fn max_runs_summary_and_saves_its_trace() {
    let corpus_dir = tempfile::tempdir().unwrap();
    let args = Args {
        harness: "fixtures/maxer/harness-with-summary/HarnessWithSummary.sol:HarnessWithSummary"
            .parse()
            .unwrap(),
        config: PathBuf::from("./ripfuzz.toml"),
        root: PathBuf::from("."),
        threads: default_threads(),
        max_runs: 64,
        max_calls: 8,
        timeout: Some(60),
        target_value: None,
        corpus_dir: corpus_dir.path().to_path_buf(),
        quiet: true,
        log_level: tracing::Level::INFO,
    };

    let best = run(args).unwrap();
    assert!(
        best.value().get() > U256::ZERO,
        "campaign must find a deposit"
    );

    assert!(
        traces_contain(".ripfuzz/traces", "summary()"),
        "a saved trace must contain the summary call"
    );
    assert!(
        traces_contain(".ripfuzz/traces", "Summarized"),
        "a saved trace must contain the summary event"
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
