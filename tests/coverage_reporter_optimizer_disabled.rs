//! Coverage reporter integration tests for the optimizer-disabled fixture.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use alloy_primitives::U256;
use alloy_sol_types::SolCall;
use raptor::{
    Artifact, ArtifactId, Chain, ChainConfig, Contract, CoverageReporter, DeployInput, Project,
    SetupInput, SharedCoverage, Transaction,
};
use revm::primitives::{Address, Bytes};

alloy_sol_types::sol! {
    interface HandlerContractBasic {
        function addAndSub(uint256 a, uint256 b) external returns (uint256);
    }

    interface HandlerContractWithLoop {
        function runLoop(uint256 count) external;
        function runNestedLoop(uint256 outer, uint256 inner) external;
    }

    interface HandlerContractWithLib {
        function libCall(uint256 amount) external returns (uint256);
    }

    interface HandlerContractWithLibLinked {
        function libLinkedCall(uint256 amount) external returns (uint256);
    }

    interface HandlerContractWithInterface {
        function interfaceCall(uint256 amount) external returns (uint256);
    }

    interface HandlerContractWithIf {
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

fn build_report(
    shared_coverage: &SharedCoverage,
    artifacts: &[Artifact],
) -> raptor::CoverageReport {
    CoverageReporter::new()
        .build_artifacts(artifacts.to_vec())
        .shared_coverage(shared_coverage.clone())
        .build()
}

const PROJECT_PATH: &str = "fixtures/coverage-reporter-optimizer-disabled";

/// Regression test: with optimizer disabled, coverage report must
/// correctly report hit counts of 1 for lines executed once.
#[test]
fn handler_contract_basic_call_once() {
    let contract = load_coverage_fixture(
        PROJECT_PATH,
        "src/HandlerContractBasic.sol:HandlerContractBasic",
    );
    let mut deployed = deploy_and_setup(PROJECT_PATH, &contract);

    let txs =
        vec![
            Transaction::new(deployed.address).calldata(Bytes::from(
                HandlerContractBasic::addAndSubCall::new((U256::from(123), U256::from(123)))
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
        "fixtures/coverage-reporter-optimizer-disabled/reports/HandlerContractBasicOnce.info";
    let expected = fs::read_to_string(expected_file)
        .unwrap_or_else(|_| panic!("expected file not found. actual output:\n{formatted}"));
    assert_eq!(
        formatted.trim(),
        expected.trim(),
        "coverage report output must match expected"
    );
}

/// Regression test: with optimizer disabled, coverage report must
/// correctly report hit counts of 2 for lines executed twice.
#[test]
fn handler_contract_basic_call_twice() {
    let contract = load_coverage_fixture(
        PROJECT_PATH,
        "src/HandlerContractBasic.sol:HandlerContractBasic",
    );
    let mut deployed = deploy_and_setup(PROJECT_PATH, &contract);

    let txs = vec![
        Transaction::new(deployed.address).calldata(Bytes::from(
            HandlerContractBasic::addAndSubCall::new((U256::from(123), U256::from(123)))
                .abi_encode(),
        )),
        Transaction::new(deployed.address).calldata(Bytes::from(
            HandlerContractBasic::addAndSubCall::new((U256::from(456), U256::from(456)))
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
        "fixtures/coverage-reporter-optimizer-disabled/reports/HandlerContractBasicTwice.info";
    let expected = fs::read_to_string(expected_file)
        .unwrap_or_else(|_| panic!("expected file not found. actual output:\n{formatted}"));
    assert_eq!(
        formatted.trim(),
        expected.trim(),
        "coverage report output must match expected"
    );
}

/// Regression test: with optimizer disabled, coverage report must
/// correctly report loop execution coverage.
#[test]
fn handler_contract_with_loop() {
    let contract = load_coverage_fixture(
        PROJECT_PATH,
        "src/HandlerContractWithLoop.sol:HandlerContractWithLoop",
    );
    let mut deployed = deploy_and_setup(PROJECT_PATH, &contract);

    let txs = vec![
        Transaction::new(deployed.address).calldata(Bytes::from(
            HandlerContractWithLoop::runLoopCall::new((U256::from(3),)).abi_encode(),
        )),
        Transaction::new(deployed.address).calldata(Bytes::from(
            HandlerContractWithLoop::runNestedLoopCall::new((U256::from(2), U256::from(2)))
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
        "fixtures/coverage-reporter-optimizer-disabled/reports/HandlerContractWithLoop.info";
    let expected = fs::read_to_string(expected_file)
        .unwrap_or_else(|_| panic!("expected file not found. actual output:\n{formatted}"));
    assert_eq!(
        formatted.trim(),
        expected.trim(),
        "coverage report output must match expected"
    );
}

/// Regression test: with optimizer disabled, coverage report must correctly
/// report internal library call coverage.
#[test]
fn handler_contract_with_lib() {
    let contract = load_coverage_fixture(
        PROJECT_PATH,
        "src/HandlerContractWithLib.sol:HandlerContractWithLib",
    );
    let mut deployed = deploy_and_setup(PROJECT_PATH, &contract);

    let txs = vec![Transaction::new(deployed.address).calldata(Bytes::from(
        HandlerContractWithLib::libCallCall::new((U256::from(123),)).abi_encode(),
    ))];
    let exec = deployed.chain.exec(&txs).unwrap();
    let coverage = exec.coverage.expect("coverage must be present");
    deployed.global.merge(&coverage);

    let project = Project::new(PROJECT_PATH);
    let artifacts: Vec<Artifact> = project.load_artifacts().unwrap().into_values().collect();
    let report = build_report(&deployed.global, &artifacts);
    let formatted = format!("{report}");

    let expected_file =
        "fixtures/coverage-reporter-optimizer-disabled/reports/HandlerContractWithLib.info";
    let expected = fs::read_to_string(expected_file)
        .unwrap_or_else(|_| panic!("expected file not found. actual output:\n{formatted}"));
    assert_eq!(
        formatted.trim(),
        expected.trim(),
        "coverage report output must match expected"
    );
}

/// Regression test: with optimizer disabled, coverage report must correctly
/// report linked library call coverage.
#[test]
fn handler_contract_with_lib_linked() {
    let contract = load_coverage_fixture(
        PROJECT_PATH,
        "src/HandlerContractWithLibLinked.sol:HandlerContractWithLibLinked",
    );
    let mut deployed = deploy_and_setup(PROJECT_PATH, &contract);

    let txs = vec![Transaction::new(deployed.address).calldata(Bytes::from(
        HandlerContractWithLibLinked::libLinkedCallCall::new((U256::from(123),)).abi_encode(),
    ))];
    let exec = deployed.chain.exec(&txs).unwrap();
    let coverage = exec.coverage.expect("coverage must be present");
    deployed.global.merge(&coverage);

    let project = Project::new(PROJECT_PATH);
    let artifacts: Vec<Artifact> = project.load_artifacts().unwrap().into_values().collect();
    let report = build_report(&deployed.global, &artifacts);
    let formatted = format!("{report}");

    let expected_file =
        "fixtures/coverage-reporter-optimizer-disabled/reports/HandlerContractWithLibLinked.info";
    let expected = fs::read_to_string(expected_file)
        .unwrap_or_else(|_| panic!("expected file not found. actual output:\n{formatted}"));
    assert_eq!(
        formatted.trim(),
        expected.trim(),
        "coverage report output must match expected"
    );
}

/// Regression test: with optimizer disabled, a deployed contract must be
/// reported correctly even when the caller interacts with it through an
/// interface.
#[test]
fn handler_contract_with_interface() {
    let contract = load_coverage_fixture(
        PROJECT_PATH,
        "src/HandlerContractWithInterface.sol:HandlerContractWithInterface",
    );
    let mut deployed = deploy_and_setup(PROJECT_PATH, &contract);

    let txs = vec![Transaction::new(deployed.address).calldata(Bytes::from(
        HandlerContractWithInterface::interfaceCallCall::new((U256::from(123),)).abi_encode(),
    ))];
    let exec = deployed.chain.exec(&txs).unwrap();
    let coverage = exec.coverage.expect("coverage must be present");
    deployed.global.merge(&coverage);

    let project = Project::new(PROJECT_PATH);
    let artifacts: Vec<Artifact> = project.load_artifacts().unwrap().into_values().collect();
    let report = build_report(&deployed.global, &artifacts);
    let formatted = format!("{report}");

    let expected_file =
        "fixtures/coverage-reporter-optimizer-disabled/reports/HandlerContractWithInterface.info";
    let expected = fs::read_to_string(expected_file)
        .unwrap_or_else(|_| panic!("expected file not found. actual output:\n{formatted}"));
    assert_eq!(
        formatted.trim(),
        expected.trim(),
        "coverage report output must match expected"
    );
}

/// Regression test: with optimizer disabled, coverage report for
/// if-statement close brackets and empty lines between if-else branches
/// must be handled correctly.
#[test]
fn handler_contract_with_if() {
    let contract = load_coverage_fixture(
        PROJECT_PATH,
        "src/HandlerContractWithIf.sol:HandlerContractWithIf",
    );
    let mut deployed = deploy_and_setup(PROJECT_PATH, &contract);

    let txs = vec![
        Transaction::new(deployed.address).calldata(Bytes::from(
            HandlerContractWithIf::runIfCall::new((true,)).abi_encode(),
        )),
        Transaction::new(deployed.address).calldata(Bytes::from(
            HandlerContractWithIf::runIfElseCall::new((true,)).abi_encode(),
        )),
        Transaction::new(deployed.address).calldata(Bytes::from(
            HandlerContractWithIf::runIfElseCall::new((false,)).abi_encode(),
        )),
        Transaction::new(deployed.address).calldata(Bytes::from(
            HandlerContractWithIf::runIfElseWithNewlineCall::new((true,)).abi_encode(),
        )),
        Transaction::new(deployed.address).calldata(Bytes::from(
            HandlerContractWithIf::runIfElseWithNewlineCall::new((false,)).abi_encode(),
        )),
        Transaction::new(deployed.address).calldata(Bytes::from(
            HandlerContractWithIf::runNestedIfCall::new((true, true)).abi_encode(),
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
        "fixtures/coverage-reporter-optimizer-disabled/reports/HandlerContractWithIf.info";
    let expected = fs::read_to_string(expected_file)
        .unwrap_or_else(|_| panic!("expected file not found. actual output:\n{formatted}"));
    assert_eq!(
        formatted.trim(),
        expected.trim(),
        "coverage report output must match expected"
    );
}
