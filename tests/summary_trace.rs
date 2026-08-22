//! A declared `summary()` function is appended to the traced re-run after
//! shrinking, so its log output shows up in the full trace.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use alloy_primitives::U256;
use ripfuzz::DEFAULT_DEPLOYER;
use ripfuzz::commands::run::{Args, run};

const PROJECT: &str = "fixtures/summary";

fn args(corpus_dir: impl AsRef<Path>) -> Args {
    Args {
        harness: "src/SummaryFail.sol:SummaryFail".to_owned(),
        project_path: Some(PathBuf::from(PROJECT)),
        deploy_value: U256::ZERO,
        deployer_address: DEFAULT_DEPLOYER,
        threads: 1,
        max_runs: 1000,
        max_failures: 1,
        timeout_secs: None,
        gas_limit: 12_500_000,
        max_calls: 4,
        seed: Some(0),
        corpus_dir: Some(corpus_dir.as_ref().to_path_buf()),
        log_level: tracing::Level::INFO,
        disable_log: false,
        ffi: false,
        force: false,
        stop_on_revert: false,
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

/// The traced re-run after shrinking must include the summary call and its
/// emitted log at the end of the sequence.
#[test]
fn summary_call_appears_in_traced_run_after_shrinking() {
    let tmp = tempfile::tempdir().unwrap();

    let before = campaign_dirs(PROJECT);
    run(args(tmp.path().join("corpus")))
        .expect("campaign must succeed when a failed assertion is found");

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

    let trace = std::fs::read_to_string(campaign_dir.join("fulltrace.log"))
        .unwrap_or_else(|_| panic!("trace file must exist in {}", campaign_dir.display()));
    assert!(
        trace.contains("summary()"),
        "trace must contain the summary call:\n{trace}"
    );
    assert!(
        trace.contains("Summarized"),
        "trace must contain the summary log:\n{trace}"
    );
    let summary_index = trace.rfind("summary()").unwrap();
    let arm_last_index = trace.rfind("invariant_never_armed()").unwrap();
    assert!(
        summary_index > arm_last_index,
        "summary call must run after the shrunk sequence:\n{trace}"
    );
}
