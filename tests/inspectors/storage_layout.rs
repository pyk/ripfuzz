use std::fs;
use std::path::{Path, PathBuf};

use ripfuzz::cli::HarnessId;
use ripfuzz::config::Config;
use ripfuzz::inspectors::StorageLayoutInspector;

const VERSION: &str = "0.8.36";

/// Copies the fixture sources into a temporary project root.
fn setup_project() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/inspectors/storage-layout/src");
    fs::create_dir_all(root.join("src")).unwrap();
    for file in ["Base.sol", "Harness.sol"] {
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
fn inspects_storage_layout_across_inheritance() {
    let (_tmp, root) = setup_project();
    let config = Config::new().with_root(&root).load("ripfuzz.toml").unwrap();
    let target = HarnessId::try_from("src/Harness.sol:Harness").unwrap();

    let output = StorageLayoutInspector::new(&root, config)
        .inspect(&target)
        .unwrap();

    assert_eq!(
        output.to_string(),
        include_str!("../../fixtures/inspectors/storage-layout/expected/report.txt")
    );
}

#[test]
fn inspect_errors_for_unknown_contract_name() {
    let (_tmp, root) = setup_project();
    let config = Config::new().with_root(&root).load("ripfuzz.toml").unwrap();
    let target = HarnessId::try_from("src/Harness.sol:Missing").unwrap();

    let err = StorageLayoutInspector::new(&root, config)
        .inspect(&target)
        .unwrap_err()
        .to_string();

    assert_eq!(
        err,
        "contract `Missing` not found in `src/Harness.sol`, available contracts: Harness"
    );
}
