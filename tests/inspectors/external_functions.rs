use std::fs;
use std::path::{Path, PathBuf};

use ripfuzz::config::Config;
use ripfuzz::harness::HarnessId;
use ripfuzz::inspectors::ExternalFunctionsInspector;

const VERSION: &str = "0.8.36";

/// Copies the fixture sources into a temporary project root.
fn setup_project() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/inspectors/external-functions/src");
    fs::create_dir_all(root.join("src")).unwrap();
    for file in ["Base.sol", "App.sol"] {
        fs::copy(fixture.join(file), root.join("src").join(file)).unwrap();
    }
    fs::write(
        root.join("ripfuzz.toml"),
        format!("[solc]\nversion = \"{VERSION}\"\n"),
    )
    .unwrap();
    (tmp, root)
}

#[test]
fn inspects_external_functions_across_inheritance() {
    let (_tmp, root) = setup_project();
    let config = Config::new().with_root(&root).load("ripfuzz.toml").unwrap();
    let target = HarnessId::try_from("src/App.sol:App").unwrap();

    let output = ExternalFunctionsInspector::new(&root, config.clone())
        .inspect(&target)
        .unwrap();

    assert_eq!(
        output.to_string(),
        include_str!("../../fixtures/inspectors/external-functions/expected/report.txt")
    );
}

#[test]
fn inspect_reuses_the_cached_compilation() {
    let (_tmp, root) = setup_project();
    let config = Config::new().with_root(&root).load("ripfuzz.toml").unwrap();
    let target = HarnessId::try_from("src/App.sol:App").unwrap();

    let first = ExternalFunctionsInspector::new(&root, config.clone())
        .inspect(&target)
        .unwrap();

    // The cache entry written by the first inspection carries the second
    // run, which must render the identical report.
    let entries: Vec<PathBuf> = fs::read_dir(root.join(".ripfuzz/solc"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    assert_eq!(entries.len(), 1, "expected one cache entry");
    let cache_path = entries[0].clone();

    let second = ExternalFunctionsInspector::new(&root, config)
        .inspect(&target)
        .unwrap();
    assert_eq!(second.to_string(), first.to_string());

    // Corrupting the cache changes the outcome, proving the cached output
    // was used instead of recompiling.
    fs::write(&cache_path, "{\"contracts\":{},\"sources\":{}}").unwrap();
    let third = ExternalFunctionsInspector::new(
        &root,
        Config::new().with_root(&root).load("ripfuzz.toml").unwrap(),
    )
    .inspect(&target);
    assert!(third.is_err());
}

#[test]
fn inspect_errors_for_unknown_contract_name() {
    let (_tmp, root) = setup_project();
    let config = Config::new().with_root(&root).load("ripfuzz.toml").unwrap();
    let target = HarnessId::try_from("src/App.sol:Missing").unwrap();

    let err = ExternalFunctionsInspector::new(&root, config)
        .inspect(&target)
        .unwrap_err()
        .to_string();

    assert_eq!(
        err,
        "contract `Missing` not found in `src/App.sol`, available contracts: App"
    );
}
