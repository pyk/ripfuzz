//! Replaying an invariant corpus that already fails an assertion must report
//! the finding without extra fuzzing runs.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use alloy_primitives::U256;
use ripfuzz::DEFAULT_DEPLOYER;
use ripfuzz::cli::run::{Args, run};

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
        quiet: true,
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

fn single_fail_args(
    corpus_dir: impl AsRef<Path>,
    max_runs: u64,
    shrink_runs: u64,
    seed: Option<u64>,
) -> Args {
    Args {
        harness: "src/SingleFail.sol:SingleFail".to_owned(),
        project_path: Some(PathBuf::from("fixtures/max-failures")),
        deploy_value: U256::ZERO,
        deployer_address: DEFAULT_DEPLOYER,
        threads: 1,
        max_runs,
        max_failures: 1,
        timeout_secs: None,
        gas_limit: 12_500_000,
        max_calls: 4,
        seed,
        corpus_dir: Some(corpus_dir.as_ref().to_path_buf()),
        log_level: tracing::Level::INFO,
        disable_log: true,
        quiet: true,
        ffi: false,
        force: false,
        stop_on_revert: false,
        external_projects: Vec::new(),
        shrink_runs,
        shrink_timeout_secs: None,
        shrink_threads: None,
    }
}

fn persisted_call_sigs(corpus_dir: &Path) -> Vec<Vec<String>> {
    let mut sigs = Vec::new();
    let mut stack = vec![corpus_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|ext| ext == "json") {
                let json: serde_json::Value =
                    serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
                sigs.push(
                    json["calls"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|call| call["sig"].as_str().unwrap().to_owned())
                        .collect(),
                );
            }
        }
    }
    sigs.sort();
    sigs
}

/// A shrunk failing sequence must be persisted to the corpus so the next
/// campaign's replay discovers the failure from the shortest calls.
#[test]
fn shrunk_failure_sequence_is_persisted_for_next_replay() {
    let tmp = tempfile::tempdir().unwrap();
    let corpus_dir = tmp.path().join("corpus");

    run(single_fail_args(&corpus_dir, 1_000, 500, Some(0)))
        .expect("campaign must find, shrink, and persist the failed assertion");

    let failing: Vec<Vec<String>> = persisted_call_sigs(&corpus_dir)
        .into_iter()
        .filter(|calls| calls.iter().any(|call| call == "invariant_never_armed()"))
        .collect();
    assert_eq!(
        failing,
        vec![vec![
            "arm()".to_owned(),
            "invariant_never_armed()".to_owned()
        ]]
    );

    // The next campaign discovers the persisted failure during replay and
    // writes its trace without fuzzing (max_runs = 0).
    let before = campaign_dirs("fixtures/max-failures");
    run(single_fail_args(&corpus_dir, 0, 1, None))
        .expect("replay of the persisted failure must succeed");
    let new_dirs: Vec<PathBuf> = campaign_dirs("fixtures/max-failures")
        .into_iter()
        .filter(|dir| !before.contains(dir))
        .collect();
    assert_eq!(
        new_dirs.len(),
        1,
        "exactly one new campaign directory expected"
    );
    assert!(
        new_dirs[0].join("fulltrace.log").is_file(),
        "replayed failed assertion must write fulltrace.log"
    );
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
    assert!(
        new_dirs[0].join("fulltrace.log").is_file(),
        "replayed failed assertion must write fulltrace.log"
    );
}
