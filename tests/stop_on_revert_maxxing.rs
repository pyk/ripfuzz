//! `--stop-on-revert` in maxxing mode: a reverted handler call stops the
//! campaign, writes the full trace to `fulltrace.log`, and dumps a compact
//! trace (call context and storage changes omitted) into the campaign log
//! and stderr instead of shrinking.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use alloy_primitives::U256;
use ripfuzz::DEFAULT_DEPLOYER;
use ripfuzz::cli::run::{Args, run};

const PROJECT: &str = "fixtures/stop-on-revert";

fn args(corpus_dir: impl AsRef<Path>) -> Args {
    Args {
        harness: "src/MaxStopOnRevert.sol:MaxStopOnRevert".to_owned(),
        project_path: Some(PathBuf::from(PROJECT)),
        deploy_value: U256::ZERO,
        deployer_address: DEFAULT_DEPLOYER,
        threads: 1,
        max_runs: 1000,
        max_failures: 25,
        timeout_secs: None,
        gas_limit: 12_500_000,
        max_calls: 4,
        seed: Some(0),
        corpus_dir: Some(corpus_dir.as_ref().to_path_buf()),
        log_level: tracing::Level::INFO,
        disable_log: false,
        quiet: true,
        ffi: false,
        force: false,
        stop_on_revert: true,
        external_projects: Vec::new(),
        shrink_runs: 500,
        shrink_timeout_secs: None,
        shrink_threads: None,
    }
}

/// Snapshot the campaign directories under the fixture project.
fn campaign_dirs(project: &str) -> HashSet<PathBuf> {
    let campaigns = Path::new(project).join(".ripfuzz").join("campaigns");
    match std::fs::read_dir(&campaigns) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect(),
        Err(_) => HashSet::new(),
    }
}

/// A reverted handler in a max-mode sequence must stop the campaign, write
/// the full trace to `fulltrace.log`, dump the compact trace into the log,
/// and name both file paths in the final error. The campaign must not
/// shrink.
#[test]
fn max_campaign_stop_on_revert_dumps_trace_into_log() {
    let tmp = tempfile::tempdir().unwrap();

    let before = campaign_dirs(PROJECT);
    let err = run(args(tmp.path().join("corpus")))
        .expect_err("campaign must fail when a transaction reverts");
    let err_str = err.to_string();
    assert!(
        err_str.contains("--stop-on-revert"),
        "error must name the trigger: {err_str}"
    );

    let new_dirs: Vec<PathBuf> = campaign_dirs(PROJECT)
        .into_iter()
        .filter(|dir| !before.contains(dir))
        .collect();
    assert_eq!(
        new_dirs.len(),
        1,
        "exactly one new campaign directory expected"
    );
    let campaign_dir = &new_dirs[0];

    assert!(
        err_str.contains("fuzz.log"),
        "error must name the log file: {err_str}"
    );
    assert!(
        err_str.contains("fulltrace.log"),
        "error must name the trace file: {err_str}"
    );

    let log = std::fs::read_to_string(campaign_dir.join("fuzz.log"))
        .unwrap_or_else(|_| panic!("campaign log must exist in {}", campaign_dir.display()));
    assert!(
        log.contains("a transaction reverted."),
        "campaign log must report the stop:\n{log}"
    );
    assert!(
        log.contains("[revert]"),
        "campaign log must contain the reverted trace:\n{log}"
    );
    assert!(
        log.contains("revert_always"),
        "campaign log must contain the reverted call:\n{log}"
    );
    assert!(
        !log.contains("call context"),
        "campaign log must carry the compact trace without call context:\n{log}"
    );

    // The trace must also be written to its own file next to the log, in
    // full with call context.
    let trace = std::fs::read_to_string(campaign_dir.join("fulltrace.log"))
        .unwrap_or_else(|_| panic!("trace file must exist in {}", campaign_dir.display()));
    assert!(
        trace.contains("[revert]"),
        "trace file must contain the reverted trace:\n{trace}"
    );
    assert!(
        trace.contains("call context"),
        "trace file must contain the full trace with call context:\n{trace}"
    );
    assert_eq!(
        trace.matches("revert_always").count(),
        1,
        "trace must stop at the first reverted call:\n{trace}"
    );

    // The stopped campaign must not shrink the max result; only the revert
    // trace file is written.
    let trace_files = std::fs::read_dir(campaign_dir)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("trace-"))
        .count();
    assert_eq!(
        trace_files,
        0,
        "no shrunk trace files expected in {}",
        campaign_dir.display()
    );
}
