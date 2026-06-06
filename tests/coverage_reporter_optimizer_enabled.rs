//! Coverage reporter integration tests for the optimizer-enabled fixture.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use alloy_primitives::U256;
use alloy_sol_types::SolCall;
use raptor::{
    Artifact, ArtifactId, Chain, ChainConfig, Contract, CoverageReport, CoverageReporter,
    DeployInput, Project, SetupInput, SharedCoverage, Transaction,
};
use revm::primitives::{Address, Bytes};

alloy_sol_types::sol! {
    interface TargetContractBasic {
        function addAndSub(uint256 a, uint256 b) external returns (uint256);
    }

    interface TargetContractWithLoop {
        function runLoop(uint256 count) external;
        function runNestedLoop(uint256 outer, uint256 inner) external;
    }

    interface TargetContractWithLib {
        function libCall(uint256 amount) external returns (uint256);
    }

    interface TargetContractWithLibLinked {
        function libLinkedCall(uint256 amount) external returns (uint256);
    }

    interface TargetContractWithInterface {
        function interfaceCall(uint256 amount) external returns (uint256);
    }

    interface TargetContractWithIf {
        function runIf(bool condition) external;
        function runIfElse(bool condition) external;
        function runIfElseWithNewline(bool condition) external;
        function runNestedIf(bool a, bool b) external;
    }
}

fn load_coverage_fixture(project_path: impl AsRef<Path>, id: &str) -> Contract {
    let project = Project::new(project_path);
    let artifacts = project.load_artifacts().unwrap();
    let artifact_id = ArtifactId::try_from(id).unwrap();
    Contract::try_get(&artifacts, &artifact_id).unwrap()
}

struct Deployed {
    chain: Chain,
    address: Address,
    global: SharedCoverage,
}

fn deploy_and_setup(project_path: impl AsRef<Path>, contract: &Contract) -> Deployed {
    let mut config = ChainConfig::default().coverage(true);
    let project = Project::new(project_path);
    let artifacts = project.load_artifacts().unwrap();
    let mut compiled_contracts = HashMap::new();
    for (id, artifact) in &artifacts {
        let initcode: Bytes = match artifact {
            Artifact::Contract(c) => c.bytecode.object.parse().unwrap_or_default(),
            Artifact::Library(c) => c.bytecode.object.parse().unwrap_or_default(),
            _ => continue,
        };
        if initcode.is_empty() {
            continue;
        }
        compiled_contracts.insert(id.into(), initcode);
    }
    config = config.with_compiled_contracts(compiled_contracts);
    let mut chain = Chain::new(config).unwrap();
    let mut deploy_opts = DeployInput::new(&contract.initcode);
    for lib in &contract.libraries {
        deploy_opts = deploy_opts.add_library(lib.clone());
    }
    let deployment = chain.deploy(deploy_opts).unwrap();
    assert!(deployment.result.success, "deployment must succeed");
    let target = deployment.address.unwrap();

    let global = SharedCoverage::new();
    global.merge(&deployment.coverage);

    if let Some(setup) = &contract.setup_function {
        let setup_data = Bytes::from(setup.selector().as_slice().to_vec());
        let setup_opts = SetupInput::new(target).calldata(setup_data);
        let setup = chain.setup(setup_opts).unwrap();
        assert!(setup.result.success, "setup must succeed");
        global.merge(&setup.coverage);
    }

    Deployed {
        chain,
        address: target,
        global,
    }
}

fn build_report(shared_coverage: &SharedCoverage, artifacts: &[Artifact]) -> CoverageReport {
    CoverageReporter::new()
        .build_artifacts(artifacts.to_vec())
        .shared_coverage(shared_coverage.clone())
        .build()
}

const PROJECT_PATH: &str = "fixtures/coverage-reporter-optimizer-enabled";

/// Regression test: with optimizer enabled, coverage report must
/// correctly report hit counts of 1 for lines executed once.
#[test]
fn target_contract_basic_call_once() {
    let contract = load_coverage_fixture(
        PROJECT_PATH,
        "src/TargetContractBasic.sol:TargetContractBasic",
    );
    let mut deployed = deploy_and_setup(PROJECT_PATH, &contract);

    let txs =
        vec![
            Transaction::new(deployed.address).calldata(Bytes::from(
                TargetContractBasic::addAndSubCall::new((U256::from(123), U256::from(123)))
                    .abi_encode(),
            )),
        ];
    let exec = deployed.chain.exec(&txs).unwrap();
    let coverage = exec.coverage.expect("coverage must be present");
    deployed.global.merge(&coverage);

    let project = Project::new(PROJECT_PATH);
    let artifacts: Vec<Artifact> = project.load_artifacts().unwrap().into_values().collect();
    let report = build_report(&deployed.global, &artifacts);
    let formatted = format!("{report}");

    let expected_file =
        "fixtures/coverage-reporter-optimizer-enabled/reports/TargetContractBasicOnce.info";
    let expected = fs::read_to_string(expected_file)
        .unwrap_or_else(|_| panic!("expected file not found. actual output:\n{formatted}"));
    assert_eq!(
        formatted.trim(),
        expected.trim(),
        "coverage report output must match expected"
    );
}

/// Regression test: with optimizer enabled, coverage report must
/// correctly report hit counts of 2 for lines executed twice.
#[test]
fn target_contract_basic_call_twice() {
    let contract = load_coverage_fixture(
        PROJECT_PATH,
        "src/TargetContractBasic.sol:TargetContractBasic",
    );
    let mut deployed = deploy_and_setup(PROJECT_PATH, &contract);

    let txs = vec![
        Transaction::new(deployed.address).calldata(Bytes::from(
            TargetContractBasic::addAndSubCall::new((U256::from(123), U256::from(123)))
                .abi_encode(),
        )),
        Transaction::new(deployed.address).calldata(Bytes::from(
            TargetContractBasic::addAndSubCall::new((U256::from(456), U256::from(456)))
                .abi_encode(),
        )),
    ];
    let exec = deployed.chain.exec(&txs).unwrap();
    let coverage = exec.coverage.expect("coverage must be present");
    deployed.global.merge(&coverage);

    let project = Project::new(PROJECT_PATH);
    let artifacts: Vec<Artifact> = project.load_artifacts().unwrap().into_values().collect();
    let report = build_report(&deployed.global, &artifacts);
    let formatted = format!("{report}");

    let expected_file =
        "fixtures/coverage-reporter-optimizer-enabled/reports/TargetContractBasicTwice.info";
    let expected = fs::read_to_string(expected_file)
        .unwrap_or_else(|_| panic!("expected file not found. actual output:\n{formatted}"));
    assert_eq!(
        formatted.trim(),
        expected.trim(),
        "coverage report output must match expected"
    );
}

/// Regression test: with optimizer enabled, coverage report must
/// correctly report loop execution coverage.
#[test]
fn target_contract_with_loop() {
    let contract = load_coverage_fixture(
        PROJECT_PATH,
        "src/TargetContractWithLoop.sol:TargetContractWithLoop",
    );
    let mut deployed = deploy_and_setup(PROJECT_PATH, &contract);

    let txs = vec![
        Transaction::new(deployed.address).calldata(Bytes::from(
            TargetContractWithLoop::runLoopCall::new((U256::from(3),)).abi_encode(),
        )),
        Transaction::new(deployed.address).calldata(Bytes::from(
            TargetContractWithLoop::runNestedLoopCall::new((U256::from(2), U256::from(2)))
                .abi_encode(),
        )),
    ];
    let exec = deployed.chain.exec(&txs).unwrap();
    let coverage = exec.coverage.expect("coverage must be present");
    deployed.global.merge(&coverage);

    let project = Project::new(PROJECT_PATH);
    let artifacts: Vec<Artifact> = project.load_artifacts().unwrap().into_values().collect();
    let report = build_report(&deployed.global, &artifacts);
    let formatted = format!("{report}");

    let expected_file =
        "fixtures/coverage-reporter-optimizer-enabled/reports/TargetContractWithLoop.info";
    let expected = fs::read_to_string(expected_file)
        .unwrap_or_else(|_| panic!("expected file not found. actual output:\n{formatted}"));
    assert_eq!(
        formatted.trim(),
        expected.trim(),
        "coverage report output must match expected"
    );
}

/// Regression test: with optimizer enabled, coverage report must correctly
/// report internal library call coverage.
#[test]
fn target_contract_with_lib() {
    let contract = load_coverage_fixture(
        PROJECT_PATH,
        "src/TargetContractWithLib.sol:TargetContractWithLib",
    );
    let mut deployed = deploy_and_setup(PROJECT_PATH, &contract);

    let txs = vec![Transaction::new(deployed.address).calldata(Bytes::from(
        TargetContractWithLib::libCallCall::new((U256::from(123),)).abi_encode(),
    ))];
    let exec = deployed.chain.exec(&txs).unwrap();
    let coverage = exec.coverage.expect("coverage must be present");
    deployed.global.merge(&coverage);

    let project = Project::new(PROJECT_PATH);
    let artifacts: Vec<Artifact> = project.load_artifacts().unwrap().into_values().collect();
    let report = build_report(&deployed.global, &artifacts);
    let formatted = format!("{report}");

    let expected_file =
        "fixtures/coverage-reporter-optimizer-enabled/reports/TargetContractWithLib.info";
    let expected = fs::read_to_string(expected_file)
        .unwrap_or_else(|_| panic!("expected file not found. actual output:\n{formatted}"));
    assert_eq!(
        formatted.trim(),
        expected.trim(),
        "coverage report output must match expected"
    );
}

/// Regression test: with optimizer enabled, coverage report must correctly
/// report linked library call coverage.
#[test]
fn target_contract_with_lib_linked() {
    let contract = load_coverage_fixture(
        PROJECT_PATH,
        "src/TargetContractWithLibLinked.sol:TargetContractWithLibLinked",
    );
    let mut deployed = deploy_and_setup(PROJECT_PATH, &contract);

    let txs = vec![Transaction::new(deployed.address).calldata(Bytes::from(
        TargetContractWithLibLinked::libLinkedCallCall::new((U256::from(123),)).abi_encode(),
    ))];
    let exec = deployed.chain.exec(&txs).unwrap();
    let coverage = exec.coverage.expect("coverage must be present");
    deployed.global.merge(&coverage);

    let project = Project::new(PROJECT_PATH);
    let artifacts: Vec<Artifact> = project.load_artifacts().unwrap().into_values().collect();
    let report = build_report(&deployed.global, &artifacts);
    let formatted = format!("{report}");

    let expected_file =
        "fixtures/coverage-reporter-optimizer-enabled/reports/TargetContractWithLibLinked.info";
    let expected = fs::read_to_string(expected_file)
        .unwrap_or_else(|_| panic!("expected file not found. actual output:\n{formatted}"));
    assert_eq!(
        formatted.trim(),
        expected.trim(),
        "coverage report output must match expected"
    );
}

/// Regression test: with optimizer enabled, a deployed contract must be
/// reported correctly even when the caller interacts with it through an
/// interface.
#[test]
fn target_contract_with_interface() {
    let contract = load_coverage_fixture(
        PROJECT_PATH,
        "src/TargetContractWithInterface.sol:TargetContractWithInterface",
    );
    let mut deployed = deploy_and_setup(PROJECT_PATH, &contract);

    let txs = vec![Transaction::new(deployed.address).calldata(Bytes::from(
        TargetContractWithInterface::interfaceCallCall::new((U256::from(123),)).abi_encode(),
    ))];
    let exec = deployed.chain.exec(&txs).unwrap();
    let coverage = exec.coverage.expect("coverage must be present");
    deployed.global.merge(&coverage);

    let project = Project::new(PROJECT_PATH);
    let artifacts: Vec<Artifact> = project.load_artifacts().unwrap().into_values().collect();
    let report = build_report(&deployed.global, &artifacts);
    let formatted = format!("{report}");

    let expected_file =
        "fixtures/coverage-reporter-optimizer-enabled/reports/TargetContractWithInterface.info";
    let expected = fs::read_to_string(expected_file)
        .unwrap_or_else(|_| panic!("expected file not found. actual output:\n{formatted}"));
    assert_eq!(
        formatted.trim(),
        expected.trim(),
        "coverage report output must match expected"
    );
}

/// Regression test: with optimizer enabled, coverage report for
/// if-statement close brackets and empty lines between if-else branches
/// must be handled correctly.
#[test]
fn target_contract_with_if() {
    let contract = load_coverage_fixture(
        PROJECT_PATH,
        "src/TargetContractWithIf.sol:TargetContractWithIf",
    );
    let mut deployed = deploy_and_setup(PROJECT_PATH, &contract);

    let txs = vec![
        Transaction::new(deployed.address).calldata(Bytes::from(
            TargetContractWithIf::runIfCall::new((true,)).abi_encode(),
        )),
        Transaction::new(deployed.address).calldata(Bytes::from(
            TargetContractWithIf::runIfElseCall::new((true,)).abi_encode(),
        )),
        Transaction::new(deployed.address).calldata(Bytes::from(
            TargetContractWithIf::runIfElseCall::new((false,)).abi_encode(),
        )),
        Transaction::new(deployed.address).calldata(Bytes::from(
            TargetContractWithIf::runIfElseWithNewlineCall::new((true,)).abi_encode(),
        )),
        Transaction::new(deployed.address).calldata(Bytes::from(
            TargetContractWithIf::runIfElseWithNewlineCall::new((false,)).abi_encode(),
        )),
        Transaction::new(deployed.address).calldata(Bytes::from(
            TargetContractWithIf::runNestedIfCall::new((true, true)).abi_encode(),
        )),
    ];
    let exec = deployed.chain.exec(&txs).unwrap();
    let coverage = exec.coverage.expect("coverage must be present");
    deployed.global.merge(&coverage);

    let project = Project::new(PROJECT_PATH);
    let artifacts: Vec<Artifact> = project.load_artifacts().unwrap().into_values().collect();
    let report = build_report(&deployed.global, &artifacts);
    let formatted = format!("{report}");

    let expected_file =
        "fixtures/coverage-reporter-optimizer-enabled/reports/TargetContractWithIf.info";
    let expected = fs::read_to_string(expected_file)
        .unwrap_or_else(|_| panic!("expected file not found. actual output:\n{formatted}"));
    assert_eq!(
        formatted.trim(),
        expected.trim(),
        "coverage report output must match expected"
    );
}
