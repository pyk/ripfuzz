//! The `fixtures/max-challenges` harnesses must reach their highest value
//! within the budget granted to their difficulty level.
//!
//! ## Layout
//!
//! - The fixtures under `fixtures/max-challenges` are sources of the project
//!   rooted at the current directory.
//! - Compilation artifacts are shared under `./.ripfuzz/out`, namespaced by
//!   the harness source path.
//! - Each challenge corpus lives in a fresh temp directory, so every run
//!   starts from an empty corpus.
//! - The failing message points at the corpus file for offline analysis.

use std::path::PathBuf;

use alloy_primitives::U256;
use ripfuzz::cli::max::{Args, run};

const MAX_CALLS: usize = 32;

/// The challenge fixtures as `(stem, contract, level)` triples.
///
/// Each `*WithNoise` variant inherits the abstract `NoiseBase` helper, so
/// the fuzzer must reach the same value as its plain counterpart while
/// `NoiseBase` handlers revert or mutate unrelated state. `NoiseBase`
/// itself is not a challenge of its own.
const CHALLENGES: &[(&str, &str, &str)] = &[
    ("Accumulate", "Accumulate", "easy"),
    ("AccumulateWithNoise", "AccumulateWithNoise", "easy"),
    ("Double", "Double", "medium"),
    ("DoubleWithNoise", "DoubleWithNoise", "medium"),
    ("Gated", "Gated", "medium"),
    ("GatedWithNoise", "GatedWithNoise", "medium"),
    ("Combo", "Combo", "hard"),
    ("ComboWithNoise", "ComboWithNoise", "hard"),
];

fn budget(level: &str) -> (usize, u64) {
    match level {
        "easy" => (4, 4096),
        "medium" => (4, 8192),
        "hard" => (4, 16384),
        other => panic!("unknown difficulty level: {other}"),
    }
}

/// The highest value reachable within the challenge budget.
fn expected_value(stem: &str) -> U256 {
    match stem {
        // The noise harness must reach the same value as the plain one.
        "Accumulate" | "AccumulateWithNoise" => U256::MAX,
        "Gated" | "GatedWithNoise" => U256::MAX,
        // The total starts at 1 and doubles once per call, so a full
        // `MAX_CALLS` sequence reaches `2 ** MAX_CALLS`.
        "Double" | "DoubleWithNoise" => U256::from(2).pow(U256::from(MAX_CALLS)),
        // The reward of 1000 is only paid when `open`, `grab`, and `claim`
        // run in that exact order.
        "Combo" | "ComboWithNoise" => U256::from(1000),
        other => panic!("challenge without an expected value: {other}"),
    }
}

/// A fresh temp corpus directory so every run starts from an empty corpus.
fn temp_corpus_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "ripfuzz-max-challenges-{}-{}",
        std::process::id(),
        fastrand::u64(..)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Every challenge harness must reach its highest value within its budget.
/// Slow: ignored by default and run explicitly with `make max`.
#[ignore = "slow fuzzing campaign; run with `make max`"]
#[test]
fn max_challenges_reach_the_highest_value() {
    let dir = PathBuf::from("fixtures/max-challenges");
    let corpus_dir = temp_corpus_dir();
    for &(stem, contract, level) in CHALLENGES {
        let path = dir.join(format!("{stem}.sol"));
        let (threads, max_runs) = budget(level);
        let expected = expected_value(stem);

        let harness = format!("{}:{}", path.display(), contract);
        let args = Args {
            harness: harness.parse().unwrap(),
            config: PathBuf::from("./ripfuzz.toml"),
            root: None,
            threads,
            max_runs,
            max_calls: MAX_CALLS,
            timeout: Some(120),
            target_value: None,
            corpus_dir: corpus_dir.clone(),
            quiet: true,
        };
        let corpus_file = corpus_dir
            .join(format!("{stem}.sol"))
            .join(contract)
            .join("corpus.json");
        let best = run(args).unwrap_or_else(|err| {
            panic!(
                "challenge {stem} failed: {err:#} (corpus {})",
                corpus_file.display()
            )
        });

        assert_eq!(
            best.value().get(),
            expected,
            "challenge {stem} did not reach the highest value within its budget (best {}, corpus {})",
            best.value(),
            corpus_file.display()
        );
    }

    let _ = std::fs::remove_dir_all(&corpus_dir);
}
