//! `--stop-on-revert` in invariant mode: a reverted handler call stops the
//! campaign and dumps the whole trace into the campaign log (both the log
//! file and stderr) instead of shrinking.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use alloy_primitives::U256;
use ripfuzz::DEFAULT_DEPLOYER;
use ripfuzz::commands::run::{Args, run};

const PROJECT: &str = "fixtures/stop-on-revert";

fn args(corpus_dir: impl AsRef<Path>) -> Args {
    Args {
        harness: "src/InvariantStopOnRevert.sol:InvariantStopOnRevert".to_owned(),
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

/// A reverted handler in an invariant-mode sequence must stop the campaign
/// and dump the whole trace into the log, without shrinking or writing trace
/// files.
#[test]
fn invariant_campaign_stop_on_revert_dumps_trace_into_log() {
    let tmp = tempfile::tempdir().unwrap();

    let before = campaign_dirs(PROJECT);
    run(args(tmp.path().join("corpus"))).expect("campaign run should succeed");

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

    let log = std::fs::read_to_string(campaign_dir.join("fuzz.log"))
        .unwrap_or_else(|_| panic!("campaign log must exist in {}", campaign_dir.display()));
    assert!(
        log.contains("[!] a transaction reverted; stopping the campaign (--stop-on-revert)"),
        "campaign log must report the stop:\n{log}"
    );
    assert!(
        log.contains("[REVERT]"),
        "campaign log must contain the reverted trace:\n{log}"
    );
    assert!(
        log.contains("revert_always"),
        "campaign log must contain the reverted call:\n{log}"
    );

    // The stopped campaign must not shrink or write trace files.
    let trace_files = std::fs::read_dir(campaign_dir)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("trace-"))
        .count();
    assert_eq!(
        trace_files,
        0,
        "no trace files expected in {}",
        campaign_dir.display()
    );
}
