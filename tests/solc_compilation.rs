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
    let harness = Solc::new()
        .with_version(VERSION)
        .with_target("fixtures/solc-compilation/HarnessWithNoImports.sol")
        .with_out(&out)
        .compile()
        .unwrap();

    assert_eq!(harness.id.name, "HarnessWithNoImports");
    assert!(harness.abi.functions.contains_key("set"));
    assert!(!harness.initcode.is_empty(), "initcode must be non-empty");

    assert!(out.exists(), "out dir must exist");

    let combined = out
        .join("fixtures/solc-compilation/HarnessWithNoImports.sol")
        .join("out.json");
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
        .join("fixtures/solc-compilation/HarnessWithNoImports.sol")
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
    let harness = Solc::new()
        .with_version(VERSION)
        .with_target("fixtures/solc-compilation/HarnessWithImports.sol")
        .with_out(&out)
        .compile()
        .unwrap();

    assert_eq!(harness.id.name, "HarnessWithImports");
    assert!(harness.abi.functions.contains_key("set"));
    assert!(!harness.initcode.is_empty(), "initcode must be non-empty");

    assert!(out.exists());

    let harness_artifact = out
        .join("fixtures/solc-compilation/HarnessWithImports.sol")
        .join("HarnessWithImports.sol")
        .join("HarnessWithImports.json");
    assert!(
        harness_artifact.is_file(),
        "harness artifact must exist at {}",
        harness_artifact.display()
    );
    assert!(contains_bytecode(&harness_artifact));

    let support_artifact = out
        .join("fixtures/solc-compilation/HarnessWithImports.sol")
        .join("Support.sol")
        .join("Support.json");
    assert!(
        support_artifact.is_file(),
        "support artifact must exist at {}",
        support_artifact.display()
    );
    assert!(contains_bytecode(&support_artifact));

    let lib_artifact = out
        .join("fixtures/solc-compilation/HarnessWithImports.sol")
        .join("Lib.sol")
        .join("Lib.json");
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
        .join("fixtures/solc-compilation/HarnessWithNoImports.sol")
        .join("HarnessWithNoImports.sol")
        .join("HarnessWithNoImports.json");
    assert!(artifact.is_file(), "artifact must be in custom dir");
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
        .join("HarnessWithNoImports.sol")
        .join("HarnessWithNoImports.json");
    assert!(
        artifact.is_file(),
        "artifact must be under the project root at {}",
        artifact.display()
    );
}

#[test]
fn compiles_harness_with_remappings() {
    let tmp = tempfile::tempdir().unwrap();
    let lib = tmp.path().join("lib/ripfuzz/src");
    fs::create_dir_all(&lib).unwrap();
    fs::write(
        lib.join("Support.sol"),
        "pragma solidity ^0.8.36;\n\ncontract Support {}",
    )
    .unwrap();
    fs::write(
        tmp.path().join("remappings.txt"),
        "ripfuzz/=lib/ripfuzz/src/\n",
    )
    .unwrap();
    fs::write(
        tmp.path().join("Harness.sol"),
        "pragma solidity ^0.8.36;\n\nimport {Support} from \"ripfuzz/Support.sol\";\n\ncontract Harness { Support public support; }",
    )
    .unwrap();

    Solc::new()
        .with_version(VERSION)
        .with_root(tmp.path())
        .with_target("Harness.sol")
        .with_out("out")
        .compile()
        .unwrap();

    let harness_artifact = tmp
        .path()
        .join("out")
        .join("Harness.sol")
        .join("Harness.sol")
        .join("Harness.json");
    assert!(harness_artifact.is_file(), "harness must compile");
    let support_artifact = tmp
        .path()
        .join("out")
        .join("Harness.sol")
        .join("Support.sol")
        .join("Support.json");
    assert!(
        support_artifact.is_file(),
        "remapped import must compile at {}",
        support_artifact.display()
    );

    let output: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(tmp.path().join("out/Harness.sol/out.json")).unwrap(),
    )
    .unwrap();
    let contracts = output.get("contracts").unwrap().as_object().unwrap();
    assert!(
        contracts.contains_key("Harness.sol"),
        "contract keys must be relative to the root, got: {:?}",
        contracts.keys().collect::<Vec<_>>()
    );
    assert!(
        contracts.contains_key("lib/ripfuzz/src/Support.sol"),
        "remapped keys must be relative to the root, got {:?}",
        contracts.keys().collect::<Vec<_>>()
    );
}

#[test]
fn with_name_selects_the_harness_contract() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(
        tmp.path().join("Two.sol"),
        "pragma solidity 0.8.36;\n\ncontract Alpha {}\n\ncontract Beta {}",
    )
    .unwrap();

    let harness = Solc::new()
        .with_version(VERSION)
        .with_root(tmp.path())
        .with_target("Two.sol")
        .with_name("Beta")
        .with_out(tmp.path().join("out"))
        .compile()
        .unwrap();

    assert_eq!(harness.id.path, Path::new("Two.sol"));
    assert_eq!(harness.id.name, "Beta");
    assert!(harness.abi.functions.is_empty());
}

#[test]
fn unknown_harness_name_fails_with_alternatives() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(
        tmp.path().join("Two.sol"),
        "pragma solidity 0.8.36;\n\ncontract Alpha {}\n\ncontract Beta {}",
    )
    .unwrap();

    let err = Solc::new()
        .with_version(VERSION)
        .with_root(tmp.path())
        .with_target("Two.sol")
        .with_name("Missing")
        .with_out(tmp.path().join("out"))
        .compile()
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "contract `Missing` not found in `Two.sol`, available contracts: Alpha, Beta"
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
