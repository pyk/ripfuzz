//! A later `ripfuzz test` campaign must not grow the corpus by re-adding
//! sequences that only rediscover already-known broken invariants.

use std::path::{Path, PathBuf};

use ripfuzz::cli::default_threads;
use ripfuzz::cli::test::{Args, run};

const HARNESS: &str = "fixtures/tester/challenges/GatedByLiterals.sol:GatedByLiterals";
const BROKEN_INVARIANT_COUNT: usize = 11;

fn args(corpus_dir: &Path) -> Args {
    Args {
        harness: HARNESS.parse().unwrap(),
        config: PathBuf::from("./ripfuzz.toml"),
        root: PathBuf::from("."),
        threads: default_threads(),
        max_fuzz_runs: 10_000,
        max_shrink_runs: 10_000,
        max_calls: 8,
        timeout: Some(120),
        max_failures: 32,
        corpus_dir: corpus_dir.to_path_buf(),
        quiet: true,
        log_level: tracing::Level::INFO,
    }
}

fn corpus_len(corpus_dir: &Path) -> usize {
    let path = corpus_dir
        .join("GatedByLiterals.sol")
        .join("GatedByLiterals")
        .join("corpus.json");
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    json["entries"].as_array().unwrap().len()
}

/// Two campaigns on `GatedByLiterals` must keep the same corpus size: the
/// second run rediscovers the same gated invariants, and those sequences
/// must not join the corpus unless they also bring new coverage.
#[test]
fn gated_by_literals_second_campaign_does_not_grow_the_corpus() {
    let tmp = tempfile::tempdir().unwrap();
    let corpus_dir = tmp.path().join("corpus");

    let first = run(args(&corpus_dir)).expect("first campaign must complete");
    assert_eq!(
        first.len(),
        BROKEN_INVARIANT_COUNT,
        "first campaign must break every gated invariant"
    );
    let first_len = corpus_len(&corpus_dir);

    let second = run(args(&corpus_dir)).expect("second campaign must complete");
    assert_eq!(
        second.len(),
        BROKEN_INVARIANT_COUNT,
        "second campaign must still report every gated invariant"
    );
    assert_eq!(
        corpus_len(&corpus_dir),
        first_len,
        "corpus must not grow when a later campaign only rediscovers known invariants"
    );
}
