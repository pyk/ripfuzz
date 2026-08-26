//! A failed harness deployment stops the campaign like `--stop-on-revert` and
//! a failed `setup()`: the full trace is written to `fulltrace.log`, a compact
//! trace (call context and storage changes omitted) goes into the campaign log
//! and stderr, and the final error names the log and trace file paths.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use alloy_primitives::U256;
use ripfuzz::DEFAULT_DEPLOYER;
use ripfuzz::commands::run::{Args, run};

const PROJECT: &str = "fixtures/basic-harness";

fn args(corpus_dir: impl AsRef<Path>) -> Args {
    Args {
        harness: "test/ConstructorRevert.sol:ConstructorRevert".to_owned(),
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

/// A reverting constructor must stop the campaign before fuzzing, write the
/// full trace to `fulltrace.log`, dump the compact trace into the log, and
/// name both file paths in the final error.
#[test]
fn deployment_failure_dumps_trace_into_log() {
    let tmp = tempfile::tempdir().unwrap();

    let before = campaign_dirs(PROJECT);
    let err = run(args(tmp.path().join("corpus")))
        .expect_err("campaign must fail when deployment reverts");
    let err_str = err.to_string();
    assert!(
        err_str.contains("harness contract deployment failed"),
        "error must name the failed deployment: {err_str}"
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
        log.contains("deployment failed."),
        "campaign log must report the failed deployment:\n{log}"
    );
    assert!(
        log.contains("[revert]"),
        "campaign log must contain the reverted trace:\n{log}"
    );
    assert!(
        log.contains("constructor always reverts"),
        "campaign log must contain the revert reason:\n{log}"
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
}
