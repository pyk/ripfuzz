//! Replaying an invariant corpus that already fails an assertion must report
//! the finding without extra fuzzing runs.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use alloy_primitives::U256;
use ripfuzz::DEFAULT_DEPLOYER;
use ripfuzz::commands::run::{Args, run};

const PROJECT: &str = "fixtures/corpus-replay";

fn args(corpus_dir: impl AsRef<Path>) -> Args {
    Args {
        harness: "src/ReplayFail.sol:ReplayFail".to_owned(),
        project_path: Some(PathBuf::from(PROJECT)),
        deploy_value: U256::ZERO,
        deployer_address: DEFAULT_DEPLOYER,
        threads: 1,
        max_runs: 0,
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
        shrink_runs: 1,
        shrink_timeout_secs: None,
        shrink_threads: None,
    }
}

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

fn write_failing_corpus(corpus_dir: &Path) {
    let namespaced = corpus_dir.join("ReplayFail.sol").join("ReplayFail");
    std::fs::create_dir_all(&namespaced).unwrap();
    std::fs::write(
        namespaced.join("fail.json"),
        r#"{
  "calls": [
    {
      "sig": "trip()",
      "args": [],
      "caller": "0xd93a248535ef447440e7d63a2aff6c3e75b235c7"
    }
  ]
}"#,
    )
    .unwrap();
}

/// A corpus item that already trips an invariant must be reported as a failed
/// assertion during replay, even when `--max-runs` is 0.
#[test]
fn invariant_replay_reports_failed_assertion_from_corpus() {
    let tmp = tempfile::tempdir().unwrap();
    let corpus_dir = tmp.path().join("corpus");
    write_failing_corpus(&corpus_dir);

    let before = campaign_dirs(PROJECT);
    run(args(&corpus_dir)).expect("campaign must succeed after reporting the replayed assertion");

    let new_dirs: Vec<PathBuf> = campaign_dirs(PROJECT)
        .into_iter()
        .filter(|dir| !before.contains(dir))
        .collect();
    assert_eq!(
        new_dirs.len(),
        1,
        "exactly one new campaign directory expected"
    );
    assert_eq!(
        new_dirs[0].join("fulltrace.log").is_file(),
        true,
        "replayed failed assertion must write fulltrace.log"
    );
}
