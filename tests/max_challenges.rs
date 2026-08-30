//! The `fixtures/max-challenges` harnesses must reach their highest value
//! within the budget granted to their difficulty level.
//!
//! The fixtures under `fixtures/max-challenges` are sources of the project
//! rooted at the current directory, and compilation artifacts are shared
//! under `./.ripfuzz/out` namespaced by the harness source path.

use std::fs;
use std::path::PathBuf;

use alloy_primitives::U256;
use ripfuzz::cli::max::{Args, run};

const MAX_CALLS: usize = 8;

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
        "easy-accumulate" => U256::MAX,
        "medium-gated" => U256::MAX,
        // The total starts at 1 and doubles once per call, so a full
        // `MAX_CALLS` sequence reaches `2 ** MAX_CALLS`.
        "medium-double" => U256::from(2).pow(U256::from(MAX_CALLS)),
        "hard-combo" => U256::from(1000),
        other => panic!("challenge without an expected value: {other}"),
    }
}

/// The contract name declared in the challenge source.
///
/// The file stem is `{level}-{name}`, and the contract is the PascalCase
/// form of the name part, e.g. `easy-accumulate` declares `Accumulate`.
fn contract_name(stem: &str) -> String {
    stem.split('-')
        .skip(1)
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

/// Every challenge harness must reach its highest value within its budget.
#[test]
fn max_challenges_reach_the_highest_value() {
    let dir = PathBuf::from("fixtures/max-challenges");
    let mut challenged = 0;
    for entry in fs::read_dir(&dir).expect("challenges dir must exist") {
        let path = entry.expect("challenge entry must exist").path();
        if path.extension().is_some_and(|ext| ext != "sol") {
            continue;
        }
        let stem = path
            .file_stem()
            .expect("challenge must have a file stem")
            .to_string_lossy()
            .to_string();
        let (level, _) = stem
            .split_once('-')
            .expect("challenge must be prefixed by level");
        let (threads, max_runs) = budget(level);
        let expected = expected_value(&stem);

        let harness = format!("{}:{}", path.display(), contract_name(&stem));
        let args = Args {
            harness: harness.parse().unwrap(),
            config: PathBuf::from("./ripfuzz.toml"),
            root: None,
            threads,
            max_runs,
            max_calls: MAX_CALLS,
            timeout: Some(120),
            target_value: None,
        };
        let best = run(args).unwrap_or_else(|err| panic!("challenge {stem} failed: {err:#}"));

        assert_eq!(
            best.value().get(),
            expected,
            "challenge {stem} did not reach the highest value within its budget (best {})",
            best.value()
        );
        challenged += 1;
    }
    assert!(challenged > 0, "no challenge fixtures found");
}
