//! A max-mode finding surfaces its trace like `--stop-on-revert` but without
//! failing the campaign: the full trace is written to `fulltrace-max-1.log`, a
//! compact trace (call context and storage changes omitted) goes into the
//! campaign log and stderr, and the log names both file paths. The campaign
//! still exits successfully.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use alloy_primitives::U256;
use ripfuzz::DEFAULT_DEPLOYER;
use ripfuzz::cli::run::{Args, run};

const PROJECT: &str = "fixtures/max-mode-harness-validation";

fn args(corpus_dir: impl AsRef<Path>) -> Args {
    Args {
        harness: "src/MaxBasic.sol:MaxBasic".to_owned(),
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

/// A max value above zero must not fail the campaign: it writes the full trace
/// to `fulltrace-max-1.log`, dumps the compact trace into the log, names both
/// file paths, and exits successfully.
#[test]
fn finding_dumps_compact_and_full_trace_into_log() {
    let tmp = tempfile::tempdir().unwrap();

    let before = campaign_dirs(PROJECT);
    run(args(tmp.path().join("corpus"))).expect("campaign must succeed when a max value is found");

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
        log.contains("max_value"),
        "campaign log must contain the max call:\n{log}"
    );
    assert!(
        log.contains("fulltrace-max-1.log"),
        "campaign log must name the trace file:\n{log}"
    );
    assert!(
        log.contains("fuzz.log"),
        "campaign log must name the log file:\n{log}"
    );
    assert!(
        !log.contains("call context"),
        "campaign log must not carry the full trace with call context:\n{log}"
    );

    // The trace must also be written to its own file next to the log, in
    // full with call context.
    let trace = std::fs::read_to_string(campaign_dir.join("fulltrace-max-1.log"))
        .unwrap_or_else(|_| panic!("trace file must exist in {}", campaign_dir.display()));
    assert!(
        trace.contains("max_value"),
        "trace file must contain the max call:\n{trace}"
    );
    assert!(
        trace.contains("call context"),
        "trace file must contain the full trace with call context:\n{trace}"
    );
}
