//! The `max` command runs the optional `summary` function on the final
//! campaign state after saving the corpus, and saves its execution trace
//! alongside the log output it emits.

use std::path::{Path, PathBuf};

use alloy_primitives::U256;
use ripfuzz::cli::max::{Args, run};

/// Snapshot the saved execution traces under the project root.
fn trace_files() -> Vec<PathBuf> {
    let traces = Path::new(".").join(".ripfuzz").join("traces");
    match std::fs::read_dir(&traces) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// The summary call must appear in a saved trace after the campaign.
#[test]
fn max_runs_summary_and_saves_its_trace() {
    let before = trace_files();

    let corpus_dir = tempfile::tempdir().unwrap();
    let args = Args {
        harness: "fixtures/max-summary-logs/HarnessWithSummary.sol:HarnessWithSummary"
            .parse()
            .unwrap(),
        config: PathBuf::from("./ripfuzz.toml"),
        root: PathBuf::from("."),
        threads: 1,
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

    let new_files: Vec<PathBuf> = trace_files()
        .into_iter()
        .filter(|path| !before.contains(path))
        .collect();
    assert_eq!(
        new_files.len(),
        1,
        "exactly one new trace file expected: {new_files:?}"
    );
    let trace = std::fs::read_to_string(&new_files[0]).unwrap();
    assert!(
        trace.contains("summary()"),
        "trace must contain the summary call:\n{trace}"
    );
    assert!(
        trace.contains("Summarized"),
        "trace must contain the summary event:\n{trace}"
    );
}
