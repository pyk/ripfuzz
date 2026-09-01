//! The `fixtures/tester/challenges` harnesses must break all their gated
//! invariants within the budget granted to their difficulty level.
//!
//! ## Layout
//!
//! - The fixtures under `fixtures/tester/challenges` are sources of the
//!   project rooted at the current directory.
//! - Compilation artifacts are shared under `./.ripfuzz/solc`, namespaced by
//!   the harness source path.
//! - Each challenge corpus lives in a fresh temp directory, so every run
//!   starts from an empty corpus.

use std::collections::HashSet;
use std::path::PathBuf;

use alloy_primitives::U256;
use ripfuzz::cli::test::{Args, run};
use ripfuzz::compilers::solc::Solc;
use ripfuzz::tester::LiteralExtractor;

const VERSION: &str = "0.8.36";
const HARNESS: &str = "fixtures/tester/challenges/GatedByLiterals.sol:GatedByLiterals";

/// The broken invariant behind every gate in `GatedByLiterals`: the bail id
/// and its description.
const BROKEN_INVARIANTS: &[(&str, &str)] = &[
    ("GATED-BOOL", "flag == true"),
    ("GATED-UINT256", "value == 2"),
    ("GATED-UINT128", "value == 12345"),
    ("GATED-INT256", "value == -7"),
    ("GATED-INT8", "value == -3"),
    ("GATED-BYTES32", "hash == 0x123456..."),
    ("GATED-BYTES1", "tag == 0xab"),
    ("GATED-ADDRESS", "account == 0x5B38..."),
    ("GATED-BYTES", "keccak256(data) == keccak256(0xdeadbeef)"),
    ("GATED-STRING", "text == gold"),
    ("GATED-ETHER", "value == 1 ether"),
];

/// Compile the challenge harness into a temporary out directory.
fn compile_harness() -> ripfuzz::compilers::solc::SolcOutput {
    let out = tempfile::tempdir().unwrap();
    Solc::new()
        .with_version(VERSION)
        .with_target("fixtures/tester/challenges/GatedByLiterals.sol")
        .with_name("GatedByLiterals")
        .with_out(&out)
        .compile()
        .expect("challenge harness must compile")
}

/// The extracted literal pools must contain the gate constant of every
/// literal kind the harness compares against.
#[test]
fn extracts_every_literal_kind_from_the_challenge() {
    let solc_output = compile_harness();
    let literals = LiteralExtractor::from_output(&solc_output.output);

    assert!(literals.bools().contains(&true));

    let two = U256::from(2);
    assert!(
        literals.uint(256).contains(&two),
        "uint256 gate literal `2`"
    );
    let twelve_thousand = U256::from(12345);
    assert!(
        literals.uint(128).contains(&twelve_thousand),
        "uint128 gate literal `12345`"
    );
    let one_ether = U256::from(1_000_000_000_000_000_000u64);
    assert!(
        literals.uint(256).contains(&one_ether),
        "`1 ether` subdenomination literal"
    );

    let negative_seven = alloy_primitives::I256::try_from(-7).unwrap();
    assert!(literals.int(256).contains(&negative_seven), "int256 `-7`");
    let negative_three = alloy_primitives::I256::try_from(-3).unwrap();
    assert!(literals.int(8).contains(&negative_three), "int8 `-3`");

    let hash_word =
        hex::decode("1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef").unwrap();
    let hash = U256::from_be_bytes::<32>(hash_word.try_into().unwrap());
    assert!(
        literals.fixed_bytes(32).contains(&hash),
        "bytes32 gate literal"
    );
    assert!(
        literals.fixed_bytes(1).contains(&U256::from(0xab)),
        "bytes1 gate literal `0xab`"
    );

    let account = "0x5B38Da6a701c568545dCfcB03FcB875f56beddC4"
        .parse::<alloy_primitives::Address>()
        .unwrap();
    assert!(literals.addresses().contains(&account));

    assert!(
        literals
            .bytes()
            .iter()
            .any(|data| data.as_ref() == [0xde, 0xad, 0xbe, 0xef]),
        "`hex\"deadbeef\"` bytes literal"
    );

    assert!(
        literals.strings().iter().any(|text| text == "gold"),
        "string gate literal `\"gold\"`"
    );
}

/// A fresh temp corpus directory so the run starts from an empty corpus.
fn temp_corpus_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "ripfuzz-tester-challenges-{}-{}",
        std::process::id(),
        fastrand::u64(..)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Every gated invariant in the challenge harness must break within the
/// easy budget. Slow: ignored by default and run explicitly with
/// `make tester-challenges`.
#[ignore = "slow fuzzing campaign; run with `make tester-challenges`"]
#[test]
fn gated_by_literals_finds_every_assertion() {
    let corpus_dir = temp_corpus_dir();
    let args = Args {
        harness: HARNESS.parse().unwrap(),
        config: PathBuf::from("./ripfuzz.toml"),
        root: PathBuf::from("."),
        threads: 4,
        max_runs: 10000,
        max_calls: 8,
        timeout: Some(120),
        max_failures: 32,
        corpus_dir: corpus_dir.clone(),
        quiet: true,
        log_level: tracing::Level::INFO,
    };

    let broken_invariants = run(args).expect("challenge should complete the campaign");

    let ids: HashSet<&str> = broken_invariants.iter().map(|broken| broken.id()).collect();
    let expected: HashSet<&str> = BROKEN_INVARIANTS.iter().map(|(id, _)| *id).collect();
    let missing: Vec<&str> = expected.difference(&ids).copied().collect();
    assert_eq!(
        ids,
        expected,
        "every gated invariant must break within its budget; missing {missing:?} (corpus {})",
        corpus_dir.display()
    );
    assert_eq!(
        broken_invariants.len(),
        BROKEN_INVARIANTS.len(),
        "every gated invariant must break exactly once (corpus {})",
        corpus_dir.display()
    );
    for broken in &broken_invariants {
        let expected = BROKEN_INVARIANTS
            .iter()
            .find(|(id, _)| *id == broken.id())
            .map(|(_, description)| *description);
        assert_eq!(
            Some(broken.description()),
            expected,
            "the description must match the bail report (corpus {})",
            corpus_dir.display()
        );
        assert_eq!(
            broken.sequence().len(),
            1,
            "the gate call must be the only call in the shrunk sequence (corpus {})",
            corpus_dir.display()
        );
    }

    let _ = std::fs::remove_dir_all(&corpus_dir);
}
