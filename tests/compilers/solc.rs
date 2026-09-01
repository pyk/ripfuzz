use std::fs;
use std::path::Path;

use ripfuzz::compilers::solc::{Solc, SolcOutput};
use ripfuzz::config::Config;
use ripfuzz::max::MaxHarness;
use solc::abi::Item;

const VERSION: &str = "0.8.36";

/// Decode the compiled target contract's initcode from the solc output.
fn initcode(solc_output: &SolcOutput) -> String {
    let contract = solc_output
        .output
        .contracts
        .get(&solc_output.id.path)
        .and_then(|contracts| contracts.get(&solc_output.id.name))
        .expect("target contract must be in the compilation output");
    contract
        .evm
        .as_ref()
        .and_then(|evm| evm.bytecode.as_ref())
        .and_then(|bytecode| bytecode.object.clone())
        .unwrap_or_default()
}

/// Decode the compiled target contract's ABI function names.
fn abi_functions(solc_output: &SolcOutput) -> Vec<String> {
    let contract = solc_output
        .output
        .contracts
        .get(&solc_output.id.path)
        .and_then(|contracts| contracts.get(&solc_output.id.name))
        .expect("target contract must be in the compilation output");
    contract
        .abi
        .iter()
        .flat_map(|abi| {
            abi.items.iter().filter_map(|item| match item {
                Item::Function(function) => Some(function.name.clone()),
                _ => None,
            })
        })
        .collect()
}

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
    let solc_output = Solc::new()
        .with_version(VERSION)
        .with_target("fixtures/compilers/solc/HarnessWithNoImports.sol")
        .with_out(&out)
        .compile()
        .unwrap();

    assert_eq!(solc_output.id.name, "HarnessWithNoImports");
    assert!(abi_functions(&solc_output).contains(&"set".to_owned()));
    assert!(
        !initcode(&solc_output).is_empty(),
        "initcode must be non-empty"
    );

    assert!(out.exists(), "out dir must exist");

    let combined = out
        .join("fixtures/compilers/solc/HarnessWithNoImports.sol")
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
        .join("fixtures/compilers/solc/HarnessWithNoImports.sol")
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
    let solc_output = Solc::new()
        .with_version(VERSION)
        .with_target("fixtures/compilers/solc/HarnessWithImports.sol")
        .with_out(&out)
        .compile()
        .unwrap();

    assert_eq!(solc_output.id.name, "HarnessWithImports");
    assert!(abi_functions(&solc_output).contains(&"set".to_owned()));
    assert!(
        !initcode(&solc_output).is_empty(),
        "initcode must be non-empty"
    );

    assert!(out.exists());

    let harness_artifact = out
        .join("fixtures/compilers/solc/HarnessWithImports.sol")
        .join("HarnessWithImports.sol")
        .join("HarnessWithImports.json");
    assert!(
        harness_artifact.is_file(),
        "harness artifact must exist at {}",
        harness_artifact.display()
    );
    assert!(contains_bytecode(&harness_artifact));

    let support_artifact = out
        .join("fixtures/compilers/solc/HarnessWithImports.sol")
        .join("Support.sol")
        .join("Support.json");
    assert!(
        support_artifact.is_file(),
        "support artifact must exist at {}",
        support_artifact.display()
    );
    assert!(contains_bytecode(&support_artifact));

    let lib_artifact = out
        .join("fixtures/compilers/solc/HarnessWithImports.sol")
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
        .with_target("fixtures/compilers/solc/HarnessWithNoImports.sol")
        .with_out(&custom)
        .compile()
        .unwrap();

    assert!(custom.exists(), "custom out must exist");
    let artifact = custom
        .join("fixtures/compilers/solc/HarnessWithNoImports.sol")
        .join("HarnessWithNoImports.sol")
        .join("HarnessWithNoImports.json");
    assert!(artifact.is_file(), "artifact must be in custom dir");
}

#[test]
fn default_out_is_dot_ripfuzz_solc() {
    let solc = Solc::new()
        .with_version(VERSION)
        .with_target("fixtures/compilers/solc/HarnessWithNoImports.sol");
    assert_eq!(solc.out_dir(), Path::new(".ripfuzz/solc"));
}

#[test]
fn with_out_overrides_default() {
    let tmp = tempfile::tempdir().unwrap();
    let custom = tmp.path().join("my_out");
    let solc = Solc::new()
        .with_version(VERSION)
        .with_target("fixtures/compilers/solc/HarnessWithNoImports.sol")
        .with_out(&custom);
    assert_eq!(solc.out_dir(), custom);
}

#[test]
fn with_root_resolves_relative_target_and_out() {
    let tmp = tempfile::tempdir().unwrap();
    fs::copy(
        "fixtures/compilers/solc/HarnessWithNoImports.sol",
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

    let solc_output = Solc::new()
        .with_version(VERSION)
        .with_root(tmp.path())
        .with_target("Two.sol")
        .with_name("Beta")
        .with_out(tmp.path().join("out"))
        .compile()
        .unwrap();

    assert_eq!(solc_output.id.path, Path::new("Two.sol"));
    assert_eq!(solc_output.id.name, "Beta");
    assert!(abi_functions(&solc_output).is_empty());
}

/// An unknown contract name must fail MaxHarness validation with the
/// available contracts listed.
#[test]
fn unknown_harness_name_fails_with_alternatives() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(
        tmp.path().join("Two.sol"),
        "pragma solidity 0.8.36;\n\ncontract Alpha {}\n\ncontract Beta {}",
    )
    .unwrap();

    let solc_output = Solc::new()
        .with_version(VERSION)
        .with_root(tmp.path())
        .with_target("Two.sol")
        .with_name("Missing")
        .with_out(tmp.path().join("out"))
        .compile()
        .unwrap();
    let err = MaxHarness::try_from(&solc_output).unwrap_err();
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
        .with_target("fixtures/compilers/solc/NonExistent.sol")
        .with_out(&out)
        .compile()
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "harness file `fixtures/compilers/solc/NonExistent.sol` not found"
    );
}

#[test]
fn missing_version_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("out");
    let err = Solc::new()
        .with_target("fixtures/compilers/solc/HarnessWithNoImports.sol")
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

/// Build a `Solc` builder from the parsed config, mirroring the wiring used
/// by `ripfuzz test`, `ripfuzz max`, and `ripfuzz exec`.
fn solc_from_config(config: &Config, target: &str, out: &Path) -> Solc {
    Solc::new()
        .with_version(&config.solc.version)
        .with_root(".")
        .with_target(target)
        .with_out(out)
        .with_evm_version(config.solc.evm_version.clone())
        .with_optimizer(config.solc.optimizer, config.solc.optimizer_runs)
        .with_via_ir(config.solc.via_ir)
        .with_remappings(config.solc.remappings.clone())
}

/// The compiled metadata records the settings solc ran with, so the config
/// values must show up there.
///
/// The raw JSON is used because the solc crate reads the `viaIR` metadata
/// key as `viaIr`, which would erase the value.
fn metadata_settings(solc_output: &SolcOutput) -> serde_json::Value {
    let contract = solc_output
        .output
        .contracts
        .get(&solc_output.id.path)
        .and_then(|contracts| contracts.get(&solc_output.id.name))
        .expect("target contract must be in the compilation output");
    let metadata = contract
        .metadata
        .as_deref()
        .expect("compiled contract must carry metadata");
    let metadata: serde_json::Value =
        serde_json::from_str(metadata).expect("metadata must be valid JSON");
    metadata
        .get("settings")
        .cloned()
        .expect("metadata must carry settings")
}

#[test]
fn config_requires_solc_version() {
    let err = Config::parse("[solc]\nout = \"out\"\n").unwrap_err();
    assert_eq!(
        err.root_cause().to_string(),
        "TOML parse error at line 1, column 1\n  |\n1 | [solc]\n  | ^^^^^^\nmissing field `version`\n"
    );

    let err = Config::parse("").unwrap_err();
    assert_eq!(
        err.root_cause().to_string(),
        "TOML parse error at line 1, column 1\n  |\n1 | \n  | ^\nmissing field `solc`\n"
    );
}

/// The legacy flat `solc = "0.8.36"` config must bail instead of being
/// silently accepted.
#[test]
fn config_rejects_legacy_flat_solc_field() {
    let err = Config::parse("solc = \"0.8.36\"\n").unwrap_err();

    assert_eq!(err.to_string(), "failed to parse config");
    assert_eq!(
        err.root_cause().to_string(),
        "TOML parse error at line 1, column 8\n  |\n1 | solc = \"0.8.36\"\n  |        ^^^^^^^^\ninvalid type: string \"0.8.36\", expected struct SolcConfig\n"
    );
}

#[test]
fn config_defaults() {
    let config = Config::parse("[solc]\nversion = \"0.8.36\"\n").unwrap();

    assert_eq!(config.solc.version, "0.8.36");
    assert_eq!(config.solc.out, Path::new(".ripfuzz/solc"));
    assert_eq!(config.solc.evm_version, solc::EvmVersion::Prague);
    assert!(!config.solc.optimizer);
    assert_eq!(config.solc.optimizer_runs, 200);
    assert!(!config.solc.via_ir);
    assert!(config.solc.remappings.is_empty());
}

/// A harness compiled from a config with every field set must run solc with
/// the configured settings, visible in the compiled metadata.
#[test]
fn compiles_harness_from_full_config() {
    let config = Config::parse(
        r#"
[solc]
version = "0.8.36"
out = ".ripfuzz/solc"
evm_version = "cancun"
optimizer = true
optimizer_runs = 200
via_ir = true
"#,
    )
    .unwrap();

    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("out");
    let solc_output = solc_from_config(
        &config,
        "fixtures/compilers/solc/HarnessWithNoImports.sol",
        &out,
    )
    .compile()
    .unwrap();

    assert_eq!(solc_output.id.name, "HarnessWithNoImports");
    assert!(!initcode(&solc_output).is_empty());

    let settings = metadata_settings(&solc_output);
    assert_eq!(settings.get("evmVersion"), Some(&"cancun".into()));
    assert_eq!(settings.get("viaIR"), Some(&true.into()));
    let optimizer = settings.get("optimizer").expect("optimizer must be set");
    assert_eq!(optimizer.get("enabled"), Some(&true.into()));
    assert_eq!(optimizer.get("runs"), Some(&200.into()));
}

/// With a minimal config the solc settings must carry the documented
/// defaults: optimizer off with 200 runs, and the Prague EVM version.
#[test]
fn compiles_harness_from_minimal_config_with_defaults() {
    let config = Config::parse("[solc]\nversion = \"0.8.36\"\n").unwrap();

    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("out");
    let solc_output = solc_from_config(
        &config,
        "fixtures/compilers/solc/HarnessWithNoImports.sol",
        &out,
    )
    .compile()
    .unwrap();

    assert_eq!(solc_output.id.name, "HarnessWithNoImports");
    let settings = metadata_settings(&solc_output);
    assert_eq!(settings.get("evmVersion"), Some(&"prague".into()));
    assert!(settings.get("viaIR").is_none());
    let optimizer = settings.get("optimizer").expect("optimizer must be set");
    assert_eq!(optimizer.get("enabled"), Some(&false.into()));
    assert_eq!(optimizer.get("runs"), Some(&200.into()));
}

/// Config remappings must resolve imports without a `remappings.txt`.
#[test]
fn compiles_harness_with_config_remappings() {
    let config = Config::parse(
        r#"
[solc]
version = "0.8.36"
remappings = ["ripfuzz/=lib/ripfuzz/src/"]
"#,
    )
    .unwrap();

    let tmp = tempfile::tempdir().unwrap();
    let lib = tmp.path().join("lib/ripfuzz/src");
    fs::create_dir_all(&lib).unwrap();
    fs::write(
        lib.join("Support.sol"),
        "pragma solidity ^0.8.36;\n\ncontract Support {}",
    )
    .unwrap();
    fs::write(
        tmp.path().join("Harness.sol"),
        "pragma solidity ^0.8.36;\n\nimport {Support} from \"ripfuzz/Support.sol\";\n\ncontract Harness { Support public support; }",
    )
    .unwrap();

    Solc::new()
        .with_version(&config.solc.version)
        .with_root(tmp.path())
        .with_target("Harness.sol")
        .with_out(tmp.path().join("out"))
        .with_remappings(config.solc.remappings.clone())
        .compile()
        .unwrap();

    let support_artifact = tmp.path().join("out/Harness.sol/Support.sol/Support.json");
    assert!(
        support_artifact.is_file(),
        "remapped import must compile at {}",
        support_artifact.display()
    );
}
