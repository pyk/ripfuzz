//! Coverage reporter integration tests against solc output.
//!
//! Compiles fixtures under `fixtures/evm/coverage-reporter` with the optimizer
//! enabled (200 runs) and asserts the full lcov report against golden files.

use std::fs;
use std::path::Path;

use alloy_primitives::U256;
use alloy_sol_types::SolCall;
use revm::primitives::{Address, Bytes};
use ripfuzz::compilers::solc::{Solc, SolcOutput};
use ripfuzz::harness::HarnessId;
use ripfuzz::{
    Chain, ChainConfig, CoverageReport, CoverageReporter, DeployInput, DeployLibraryInput,
    SetupInput, SharedCoverage, Transaction,
};

const ROOT: &str = "fixtures/evm/coverage-reporter";
const VERSION: &str = "0.8.36";

alloy_sol_types::sol! {
    interface HarnessContractBasic {
        function addAndSub(uint256 a, uint256 b) external returns (uint256);
    }

    interface HarnessContractWithLoop {
        function setup() external;
        function runLoop(uint256 count) external;
        function runNestedLoop(uint256 outer, uint256 inner) external;
    }

    interface HarnessContractWithLib {
        function libCall(uint256 amount) external returns (uint256);
    }

    interface HarnessContractWithLibLinked {
        function libLinkedCall(uint256 amount) external returns (uint256);
    }

    interface HarnessContractWithInterface {
        function interfaceCall(uint256 amount) external returns (uint256);
    }

    interface HarnessContractWithIf {
        function runIf(bool condition) external;
        function runIfElse(bool condition) external;
        function runIfElseWithNewline(bool condition) external;
        function runNestedIf(bool a, bool b) external;
    }
}

struct Deployed {
    chain: Chain,
    address: Address,
    global: SharedCoverage,
}

fn compile_fixture(target: &str) -> SolcOutput {
    let id = HarnessId::try_from(target).unwrap();
    let tmp = tempfile::tempdir().unwrap();
    Solc::new()
        .with_version(VERSION)
        .with_root(ROOT)
        .with_target(&id.path)
        .with_name(&id.name)
        .with_out(tmp.path().join("out"))
        .with_optimizer(true, 200)
        .compile()
        .unwrap_or_else(|err| panic!("fixture `{target}` must compile: {err}"))
}

fn contract_initcode(output: &SolcOutput, source: &str, name: &str) -> String {
    output
        .output
        .contracts
        .get(Path::new(source))
        .and_then(|contracts| contracts.get(name))
        .and_then(|contract| contract.evm.as_ref())
        .and_then(|evm| evm.bytecode.as_ref())
        .and_then(|bytecode| bytecode.object.clone())
        .unwrap_or_else(|| panic!("initcode `{source}:{name}` must be present"))
}

fn deploy(initcode: &str, libraries: Vec<DeployLibraryInput>) -> Deployed {
    let config = ChainConfig::default().coverage(true);
    let mut chain = Chain::new(config).unwrap();
    let mut deploy_opts = DeployInput::new(initcode);
    for library in libraries {
        deploy_opts = deploy_opts.add_library(library);
    }
    let deployment = chain.deploy(deploy_opts).unwrap();
    assert!(deployment.result.success, "deployment must succeed");
    let address = deployment.address.unwrap();

    let global = SharedCoverage::new();
    global.merge(&deployment.coverage);

    Deployed {
        chain,
        address,
        global,
    }
}

fn build_report(shared_coverage: &SharedCoverage, solc_output: &SolcOutput) -> CoverageReport {
    CoverageReporter::new()
        .solc_output(solc_output)
        .shared_coverage(shared_coverage.clone())
        .base_project_path(ROOT)
        .build()
}

fn assert_report(report: &CoverageReport, expected_file: &str) {
    let formatted = format!("{report}");
    let expected = fs::read_to_string(expected_file)
        .unwrap_or_else(|_| panic!("expected file not found. actual output:\n{formatted}"));
    assert_eq!(
        formatted.trim(),
        expected.trim(),
        "coverage report output must match expected"
    );
}

#[test]
fn harness_contract_basic_call_once() {
    let solc_output = compile_fixture("HarnessContractBasic.sol:HarnessContractBasic");
    let mut deployed = deploy(solc_output.initcode().unwrap(), Vec::new());

    let txs =
        vec![
            Transaction::new(deployed.address).calldata(Bytes::from(
                HarnessContractBasic::addAndSubCall::new((U256::from(123), U256::from(123)))
                    .abi_encode(),
            )),
        ];
    let exec = deployed.chain.exec(&txs).unwrap();
    let coverage = exec.coverage.expect("coverage must be present");
    deployed.global.merge(&coverage);

    let report = build_report(&deployed.global, &solc_output);
    assert_report(
        &report,
        "fixtures/evm/coverage-reporter/expected/HarnessContractBasicOnce.info",
    );
}

#[test]
fn harness_contract_basic_call_twice() {
    let solc_output = compile_fixture("HarnessContractBasic.sol:HarnessContractBasic");
    let mut deployed = deploy(solc_output.initcode().unwrap(), Vec::new());

    let txs = vec![
        Transaction::new(deployed.address).calldata(Bytes::from(
            HarnessContractBasic::addAndSubCall::new((U256::from(123), U256::from(123)))
                .abi_encode(),
        )),
        Transaction::new(deployed.address).calldata(Bytes::from(
            HarnessContractBasic::addAndSubCall::new((U256::from(456), U256::from(456)))
                .abi_encode(),
        )),
    ];
    let exec = deployed.chain.exec(&txs).unwrap();
    let coverage = exec.coverage.expect("coverage must be present");
    deployed.global.merge(&coverage);

    let report = build_report(&deployed.global, &solc_output);
    assert_report(
        &report,
        "fixtures/evm/coverage-reporter/expected/HarnessContractBasicTwice.info",
    );
}

#[test]
fn harness_contract_with_loop() {
    let solc_output = compile_fixture("HarnessContractWithLoop.sol:HarnessContractWithLoop");
    let mut deployed = deploy(solc_output.initcode().unwrap(), Vec::new());

    let setup = SetupInput::new(deployed.address).calldata(Bytes::from(
        HarnessContractWithLoop::setupCall::new(()).abi_encode(),
    ));
    let setup = deployed.chain.setup(setup).unwrap();
    assert!(setup.result.success, "setup must succeed");
    deployed.global.merge(&setup.coverage);

    let txs = vec![
        Transaction::new(deployed.address).calldata(Bytes::from(
            HarnessContractWithLoop::runLoopCall::new((U256::from(3),)).abi_encode(),
        )),
        Transaction::new(deployed.address).calldata(Bytes::from(
            HarnessContractWithLoop::runNestedLoopCall::new((U256::from(2), U256::from(2)))
                .abi_encode(),
        )),
    ];
    let exec = deployed.chain.exec(&txs).unwrap();
    let coverage = exec.coverage.expect("coverage must be present");
    deployed.global.merge(&coverage);

    let report = build_report(&deployed.global, &solc_output);
    assert_report(
        &report,
        "fixtures/evm/coverage-reporter/expected/HarnessContractWithLoop.info",
    );
}

#[test]
fn harness_contract_with_lib() {
    let solc_output = compile_fixture("HarnessContractWithLib.sol:HarnessContractWithLib");
    let mut deployed = deploy(solc_output.initcode().unwrap(), Vec::new());

    let txs = vec![Transaction::new(deployed.address).calldata(Bytes::from(
        HarnessContractWithLib::libCallCall::new((U256::from(123),)).abi_encode(),
    ))];
    let exec = deployed.chain.exec(&txs).unwrap();
    let coverage = exec.coverage.expect("coverage must be present");
    deployed.global.merge(&coverage);

    let report = build_report(&deployed.global, &solc_output);
    assert_report(
        &report,
        "fixtures/evm/coverage-reporter/expected/HarnessContractWithLib.info",
    );
}

#[test]
fn harness_contract_with_lib_linked() {
    let solc_output =
        compile_fixture("HarnessContractWithLibLinked.sol:HarnessContractWithLibLinked");
    let library = DeployLibraryInput::new(
        "MathLibLinked.sol:MathLibLinked",
        &contract_initcode(&solc_output, "MathLibLinked.sol", "MathLibLinked"),
    );
    let mut deployed = deploy(solc_output.initcode().unwrap(), vec![library]);

    let txs = vec![Transaction::new(deployed.address).calldata(Bytes::from(
        HarnessContractWithLibLinked::libLinkedCallCall::new((U256::from(123),)).abi_encode(),
    ))];
    let exec = deployed.chain.exec(&txs).unwrap();
    let coverage = exec.coverage.expect("coverage must be present");
    deployed.global.merge(&coverage);

    let report = build_report(&deployed.global, &solc_output);
    assert_report(
        &report,
        "fixtures/evm/coverage-reporter/expected/HarnessContractWithLibLinked.info",
    );
}

#[test]
fn harness_contract_with_interface() {
    let solc_output =
        compile_fixture("HarnessContractWithInterface.sol:HarnessContractWithInterface");
    let mut deployed = deploy(solc_output.initcode().unwrap(), Vec::new());

    let txs = vec![Transaction::new(deployed.address).calldata(Bytes::from(
        HarnessContractWithInterface::interfaceCallCall::new((U256::from(123),)).abi_encode(),
    ))];
    let exec = deployed.chain.exec(&txs).unwrap();
    let coverage = exec.coverage.expect("coverage must be present");
    deployed.global.merge(&coverage);

    let report = build_report(&deployed.global, &solc_output);
    assert_report(
        &report,
        "fixtures/evm/coverage-reporter/expected/HarnessContractWithInterface.info",
    );
}

#[test]
fn harness_contract_with_if() {
    let solc_output = compile_fixture("HarnessContractWithIf.sol:HarnessContractWithIf");
    let mut deployed = deploy(solc_output.initcode().unwrap(), Vec::new());

    let txs = vec![
        Transaction::new(deployed.address).calldata(Bytes::from(
            HarnessContractWithIf::runIfCall::new((true,)).abi_encode(),
        )),
        Transaction::new(deployed.address).calldata(Bytes::from(
            HarnessContractWithIf::runIfElseCall::new((true,)).abi_encode(),
        )),
        Transaction::new(deployed.address).calldata(Bytes::from(
            HarnessContractWithIf::runIfElseCall::new((false,)).abi_encode(),
        )),
        Transaction::new(deployed.address).calldata(Bytes::from(
            HarnessContractWithIf::runIfElseWithNewlineCall::new((true,)).abi_encode(),
        )),
        Transaction::new(deployed.address).calldata(Bytes::from(
            HarnessContractWithIf::runIfElseWithNewlineCall::new((false,)).abi_encode(),
        )),
        Transaction::new(deployed.address).calldata(Bytes::from(
            HarnessContractWithIf::runNestedIfCall::new((true, true)).abi_encode(),
        )),
    ];
    let exec = deployed.chain.exec(&txs).unwrap();
    let coverage = exec.coverage.expect("coverage must be present");
    deployed.global.merge(&coverage);

    let report = build_report(&deployed.global, &solc_output);
    assert_report(
        &report,
        "fixtures/evm/coverage-reporter/expected/HarnessContractWithIf.info",
    );
}
