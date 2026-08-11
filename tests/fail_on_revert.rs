//! `--fail-on-revert` across both campaign modes: invariant and
//! maxxing.
//!
//! A reverted handler call must be reported as a failed assertion, shrunk,
//! and traced, and the campaign must stop at the first failure regardless of
//! `--max-failures`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use alloy_primitives::U256;
use ripfuzz::DEFAULT_DEPLOYER;
use ripfuzz::commands::run::{Args, run};

const PROJECT: &str = "fixtures/fail-on-revert";

/// Serialize campaign runs so the two tests never race on the fixture's
/// `.ripfuzz/campaigns/` directory and each one observes exactly its own
/// campaign directory.
static CAMPAIGN_LOCK: Mutex<()> = Mutex::new(());

fn base_args(corpus_dir: impl AsRef<Path>) -> Args {
    let corpus_dir = corpus_dir.as_ref().to_path_buf();
    Args {
        harness: String::new(),
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
        corpus_dir: Some(corpus_dir),
        log_level: tracing::Level::INFO,
        disable_log: true,
        ffi: false,
        force: false,
        fail_on_revert: true,
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

/// Run a campaign and return the campaign directory it created.
fn run_and_get_campaign_dir(args: Args) -> PathBuf {
    let before = campaign_dirs(PROJECT);
    run(args).expect("campaign run should succeed");

    let new_dirs: Vec<PathBuf> = campaign_dirs(PROJECT)
        .into_iter()
        .filter(|dir| !before.contains(dir))
        .collect();
    assert_eq!(
        new_dirs.len(),
        1,
        "exactly one new campaign directory expected"
    );
    new_dirs[0].clone()
}

/// A reverted handler in a max-mode sequence must be reported as a failed
/// assertion and traced, and the campaign must stop at the first failure even
/// when `--max-failures` is larger.
#[test]
fn max_campaign_fail_on_revert_reports_reverted_handler() {
    let _guard = CAMPAIGN_LOCK.lock().unwrap();
    let tmp = tempfile::tempdir().unwrap();

    let mut args = base_args(tmp.path().join("corpus"));
    args.harness = "src/MaxFailOnRevert.sol:MaxFailOnRevert".to_owned();

    let campaign_dir = run_and_get_campaign_dir(args);

    assert_eq!(
        campaign_dir.join("trace-max-fail.log").is_file(),
        true,
        "failure trace must be written in {}",
        campaign_dir.display()
    );
    assert_eq!(
        campaign_dir.join("trace-max-fail-2.log").exists(),
        false,
        "the campaign must stop at the first failure despite --max-failures 25"
    );
}

/// A reverted handler in an invariant-mode sequence must be reported as a
/// failed assertion and traced, and the campaign must stop at the first
/// failure even when `--max-failures` is larger.
#[test]
fn invariant_campaign_fail_on_revert_reports_reverted_handler() {
    let _guard = CAMPAIGN_LOCK.lock().unwrap();
    let tmp = tempfile::tempdir().unwrap();

    let mut args = base_args(tmp.path().join("corpus"));
    args.harness = "src/InvariantFailOnRevert.sol:InvariantFailOnRevert".to_owned();

    let campaign_dir = run_and_get_campaign_dir(args);

    assert_eq!(
        campaign_dir.join("trace.log").is_file(),
        true,
        "failure trace must be written in {}",
        campaign_dir.display()
    );
    assert_eq!(
        campaign_dir.join("trace-2.log").exists(),
        false,
        "the campaign must stop at the first failure despite --max-failures 25"
    );
}
