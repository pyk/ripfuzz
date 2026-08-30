use std::fs;
use std::path::Path;

use ripfuzz::solc::Solc;

const VERSION: &str = "0.8.36";

fn contains_bytecode(path: &Path) -> bool {
    let content = fs::read_to_string(path).unwrap();
    let v: serde_json::Value = serde_json::from_str(&content).unwrap();
    let bytecode = v
        .get("evm")
        .and_then(|evm| evm.get("bytecode"))
        .and_then(|b| b.get("object"))
        .and_then(|o| o.as_str())
        .or_else(|| {
            v.get("bytecode")
                .and_then(|b| b.get("object"))
                .and_then(|o| o.as_str())
        })
        .unwrap_or("");
    !bytecode.is_empty() && bytecode != "0x"
}

#[test]
fn compiles_harness_with_no_imports() {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("out");
    Solc::new()
        .with_version(VERSION)
        .with_target("fixtures/solc-compilation/HarnessWithNoImports.sol")
        .with_out(&out)
        .compile()
        .unwrap();

    assert!(out.exists(), "out dir must exist");

    let combined = out.join("output.json");
    assert!(
        combined.is_file(),
        "combined output must exist at {}",
        combined.display()
    );
    let output: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&combined).unwrap()).unwrap();
    let contracts = output.get("contracts").expect("contracts must exist");
    assert!(
        !contracts.as_object().unwrap().is_empty(),
        "contracts must not be empty"
    );

    let artifact = out
        .join("HarnessWithNoImports.sol")
        .join("HarnessWithNoImports.json");
    assert!(
        artifact.is_file(),
        "artifact must exist at {}",
        artifact.display()
    );
    let v: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&artifact).unwrap()).unwrap();
    assert!(v.get("abi").is_some(), "abi must exist");
    assert!(contains_bytecode(&artifact), "bytecode must be non-empty");
}

#[test]
fn compiles_harness_with_imports() {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("out");
    Solc::new()
        .with_version(VERSION)
        .with_target("fixtures/solc-compilation/HarnessWithImports.sol")
        .with_out(&out)
        .compile()
        .unwrap();

    assert!(out.exists());

    let harness_artifact = out
        .join("HarnessWithImports.sol")
        .join("HarnessWithImports.json");
    assert!(
        harness_artifact.is_file(),
        "harness artifact must exist at {}",
        harness_artifact.display()
    );
    assert!(contains_bytecode(&harness_artifact));

    let support_artifact = out.join("Support.sol").join("Support.json");
    assert!(
        support_artifact.is_file(),
        "support artifact must exist at {}",
        support_artifact.display()
    );
    assert!(contains_bytecode(&support_artifact));

    let lib_artifact = out.join("Lib.sol").join("Lib.json");
    assert!(
        lib_artifact.is_file(),
        "lib artifact must exist at {}",
        lib_artifact.display()
    );
    let lib_v: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&lib_artifact).unwrap()).unwrap();
    assert!(lib_v.get("abi").is_some());
}

#[test]
fn with_out_uses_custom_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let custom = tmp.path().join("custom_out");
    Solc::new()
        .with_version(VERSION)
        .with_target("fixtures/solc-compilation/HarnessWithNoImports.sol")
        .with_out(&custom)
        .compile()
        .unwrap();

    assert!(custom.exists(), "custom out must exist");
    let artifact = custom
        .join("HarnessWithNoImports.sol")
        .join("HarnessWithNoImports.json");
    assert!(artifact.is_file(), "artifact must be in custom dir");

    let default_out = Path::new(".ripfuzz/out");
    if default_out.exists() {
        let maybe = default_out
            .join("HarnessWithNoImports.sol")
            .join("HarnessWithNoImports.json");
        if maybe.is_file() {
            let _ = fs::remove_file(maybe);
        }
    }
}

#[test]
fn default_out_is_dot_ripfuzz_out() {
    let solc = Solc::new()
        .with_version(VERSION)
        .with_target("fixtures/solc-compilation/HarnessWithNoImports.sol");
    assert_eq!(solc.out_dir(), Path::new(".ripfuzz/out"));
}

#[test]
fn with_out_overrides_default() {
    let tmp = tempfile::tempdir().unwrap();
    let custom = tmp.path().join("my_out");
    let solc = Solc::new()
        .with_version(VERSION)
        .with_target("fixtures/solc-compilation/HarnessWithNoImports.sol")
        .with_out(&custom);
    assert_eq!(solc.out_dir(), custom);
}

#[test]
fn with_root_resolves_relative_target_and_out() {
    let tmp = tempfile::tempdir().unwrap();
    fs::copy(
        "fixtures/solc-compilation/HarnessWithNoImports.sol",
        tmp.path().join("HarnessWithNoImports.sol"),
    )
    .unwrap();

    Solc::new()
        .with_version(VERSION)
        .with_root(tmp.path())
        .with_target("HarnessWithNoImports.sol")
        .with_out(".ripfuzz/out")
        .compile()
        .unwrap();

    let artifact = tmp
        .path()
        .join(".ripfuzz")
        .join("out")
        .join("HarnessWithNoImports.sol")
        .join("HarnessWithNoImports.json");
    assert!(
        artifact.is_file(),
        "artifact must be under the project root at {}",
        artifact.display()
    );
}

#[test]
fn missing_target_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("out");
    let err = Solc::new()
        .with_version(VERSION)
        .with_target("fixtures/solc-compilation/NonExistent.sol")
        .with_out(&out)
        .compile()
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "harness file `fixtures/solc-compilation/NonExistent.sol` not found"
    );
}

#[test]
fn missing_version_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("out");
    let err = Solc::new()
        .with_target("fixtures/solc-compilation/HarnessWithNoImports.sol")
        .with_out(&out)
        .compile()
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "solc version not set, call Solc::new().with_version(..)"
    );
}

#[test]
fn missing_target_not_set_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("out");
    let err = Solc::new()
        .with_version(VERSION)
        .with_out(&out)
        .compile()
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "solc target not set, call Solc::new().with_target(..)"
    );
}

#[test]
fn invalid_sol_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("src");
    fs::create_dir_all(&dir).unwrap();
    let bad = dir.join("Bad.sol");
    fs::write(
        &bad,
        "pragma solidity ^0.8.36; contract Bad { invalid syntax }",
    )
    .unwrap();
    let out = tmp.path().join("out");
    let err = Solc::new()
        .with_version(VERSION)
        .with_target(&bad)
        .with_out(&out)
        .compile()
        .unwrap_err();
    assert!(
        err.to_string().contains("solc compilation failed"),
        "got: {}",
        err
    );
}
