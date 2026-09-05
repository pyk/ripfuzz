use std::fs;
use std::path::{Path, PathBuf};

use ripfuzz::config::Config;
use ripfuzz::harness::HarnessId;
use ripfuzz::inspectors::FunctionSourceInspector;

const VERSION: &str = "0.8.36";

/// Copies the fixture sources into a temporary project root.
fn setup_project() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/inspectors/function-source/src");
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

/// Inspects `selector` on the fixture app, returning the rendered report.
fn inspect(root: &Path, selector: &str) -> String {
    let config = Config::new().with_root(root).load("ripfuzz.toml").unwrap();
    let target = HarnessId::try_from("src/App.sol:App").unwrap();
    FunctionSourceInspector::new(root, config)
        .inspect(&target, selector)
        .map(|output| format!("{output}\n"))
        .unwrap_or_else(|error| panic!("{error}"))
}

/// The `mid` report carries the function, its internal callee, the struct
/// parameter, the scaling constant, and the custom error it reverts with.
#[test]
fn inspect_renders_the_function_with_every_referenced_symbol() {
    let (_tmp, root) = setup_project();

    let output = inspect(&root, "3da89302");

    assert_eq!(
        output,
        include_str!("../../fixtures/inspectors/function-source/expected/mid.txt")
    );
}

/// An inherited report resolves the base declaration and the public getter
/// the function reads.
#[test]
fn inspect_resolves_inherited_symbols() {
    let (_tmp, root) = setup_project();

    let output = inspect(&root, "57de26a4");

    assert_eq!(
        output,
        include_str!("../../fixtures/inspectors/function-source/expected/read.txt")
    );
}

/// The `configure` report carries the modifier, the emitted event, the
/// state variable writes, and the inherited getter it reads.
#[test]
fn inspect_resolves_modifier_event_and_inherited_state() {
    let (_tmp, root) = setup_project();

    let output = inspect(&root, "75cb2672");

    assert_eq!(
        output,
        include_str!("../../fixtures/inspectors/function-source/expected/configure.txt")
    );
}

/// An unknown selector fails with the sorted selector table of the contract.
#[test]
fn inspect_errors_for_an_unknown_selector() {
    let (_tmp, root) = setup_project();

    let config = Config::new().with_root(&root).load("ripfuzz.toml").unwrap();
    let target = HarnessId::try_from("src/App.sol:App").unwrap();
    let error = FunctionSourceInspector::new(&root, config)
        .inspect(&target, "deadbeef")
        .unwrap_err()
        .to_string();

    assert_eq!(
        format!("{error}\n"),
        include_str!("../../fixtures/inspectors/function-source/expected/unknown_selector.txt")
    );
}
