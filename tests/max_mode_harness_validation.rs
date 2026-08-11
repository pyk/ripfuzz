//! Max mode harness validation: automatic mode selection and clear errors
//! for invalid max-mode harnesses.

use std::path::{Path, PathBuf};

use alloy_primitives::U256;
use ripfuzz::DEFAULT_DEPLOYER;
use ripfuzz::commands::run::{Args, run};

const PROJECT: &str = "fixtures/max-mode-harness-validation";

fn count_corpus_files(dir: impl AsRef<Path>) -> usize {
    let dir = dir.as_ref();
    if !dir.exists() {
        return 0;
    }
    walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension() == Some("json".as_ref()))
        .count()
}

fn make_args(corpus_dir: impl AsRef<Path>) -> Args {
    let corpus_dir = corpus_dir.as_ref().to_path_buf();
    Args {
        harness: "src/MaxBasic.sol:MaxBasic".to_owned(),
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
        corpus_dir: Some(corpus_dir),
        log_level: tracing::Level::INFO,
        disable_log: true,
        ffi: false,
        force: false,
        stop_on_revert: false,
        external_projects: Vec::new(),
        shrink_runs: 500,
        shrink_timeout_secs: None,
        shrink_threads: None,
    }
}

/// A harness with a `max_*` function must enter max mode automatically and
/// persist the improving sequence to the corpus.
#[test]
fn max_mode_is_entered_automatically() {
    let tmp = tempfile::tempdir().unwrap();
    let corpus_dir = tmp.path().join("corpus");

    let args = make_args(&corpus_dir);
    run(args).expect("auto-detected max mode run should succeed");
    assert!(
        count_corpus_files(&corpus_dir) > 0,
        "max mode should persist improving sequences to the corpus"
    );
}

/// Max mode must reject a harness that also declares `invariant_*` functions
/// with a clear error.
#[test]
fn max_mode_rejects_invariant_functions() {
    let tmp = tempfile::tempdir().unwrap();

    let mut args = make_args(tmp.path().join("corpus"));
    args.harness = "src/MaxMixed.sol:MaxMixed".to_owned();
    args.max_runs = 200;
    args.shrink_runs = 100;

    let err = run(args).expect_err("max mode with invariants must fail");
    assert!(
        err.to_string()
            .contains("max mode does not support invariants"),
        "unexpected error: {err}"
    );
    assert!(
        err.to_string().contains("`invariant_value_is_zero`"),
        "error must name the invariant function: {err}"
    );
}

/// Max mode must reject a harness with multiple `max_*` functions with a
/// clear error.
#[test]
fn max_mode_rejects_multiple_max_functions() {
    let tmp = tempfile::tempdir().unwrap();

    let mut args = make_args(tmp.path().join("corpus"));
    args.harness = "src/MaxMultiple.sol:MaxMultiple".to_owned();
    args.max_runs = 200;
    args.shrink_runs = 100;

    let err = run(args).expect_err("max mode with multiple max functions must fail");
    assert!(
        err.to_string()
            .contains("max mode supports exactly one `max_*` function"),
        "unexpected error: {err}"
    );
    assert!(
        err.to_string().contains("`max_a`") && err.to_string().contains("`max_b`"),
        "error must name all max functions: {err}"
    );
}
