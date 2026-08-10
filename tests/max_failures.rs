//! `--max-failures` tests for single, duplicate, and multiple failed
//! assertions.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use alloy_primitives::{Address, B256};
use ripfuzz::{
    ArtifactId, Chain, ChainConfig, Contract, CorpusConfig, DEFAULT_DEPLOYER, DeployInput,
    FailedAssertion, Fuzzer, FuzzerConfig, Item, Project, SharedCorpus, SharedCoverage,
    SharedFailedAssertions, SharedFailedCorpusItem, SharedMetrics, Shrinker, ShrinkerConfig,
    Transaction,
};

const PROJECT: &str = "fixtures/max-failures";

fn load_contract(id: &str) -> Contract {
    let project = Project::new(PROJECT);
    let artifacts = project.load_artifacts().unwrap();
    let artifact_id = ArtifactId::try_from(id).unwrap();
    Contract::try_get(&artifacts, &artifact_id).unwrap()
}

fn deploy_contract(contract: &Contract) -> (Chain, Address) {
    let mut chain = Chain::new(ChainConfig::default().coverage(true)).unwrap();
    let deployment = chain.deploy(DeployInput::new(&contract.initcode)).unwrap();
    assert!(deployment.result.success, "deployment must succeed");
    let target = deployment.address.unwrap();

    if let Some(setup) = &contract.setup_function {
        let calldata = setup.selector().as_slice().to_vec();
        let setup = chain
            .setup(
                ripfuzz::SetupInput::new(target)
                    .calldata(calldata.into())
                    .caller(DEFAULT_DEPLOYER),
            )
            .unwrap();
        assert!(setup.result.success, "setup must succeed");
    }

    (chain, target)
}

fn signatures(contract: &Contract) -> Vec<String> {
    contract
        .handler_functions
        .iter()
        .chain(contract.invariant_functions.iter())
        .map(|f| f.signature())
        .collect()
}

fn run_fuzzer(
    contract_id: &str,
    max_failures: usize,
    max_calls: usize,
    max_runs: u64,
    seed: u64,
) -> (Arc<AtomicBool>, Vec<FailedAssertion>) {
    let tmp = tempfile::tempdir().unwrap();
    let contract = load_contract(contract_id);
    let (chain, target) = deploy_contract(&contract);

    let corpus = SharedCorpus::new(
        CorpusConfig::new(tmp.path().join("corpus"))
            .handler_functions(contract.handler_functions.clone())
            .max_calls(max_calls),
    );
    let shared_metrics = SharedMetrics::new(signatures(&contract));
    let shared_failed_assertions = SharedFailedAssertions::new(max_failures);
    let shutdown = Arc::new(AtomicBool::new(false));

    let config = FuzzerConfig::new()
        .chain(chain)
        .target_address(target)
        .shared_corpus(corpus)
        .shared_coverage(SharedCoverage::new())
        .shared_metrics(shared_metrics)
        .shared_failed_assertions(shared_failed_assertions.clone())
        .shutdown_signal(shutdown.clone())
        .invariant_functions(contract.invariant_functions.clone())
        .caller(DEFAULT_DEPLOYER)
        .gas_limit(12_500_000)
        .max_runs(max_runs)
        .seed(seed)
        .fail_on_revert(false);

    let fuzzer = Fuzzer::new(config);
    fuzzer.run().unwrap();

    (shutdown, shared_failed_assertions.items())
}

fn assert_still_fails(chain: &Chain, target: Address, item: &Item) {
    let transactions: Vec<Transaction> = item
        .calls
        .iter()
        .map(|call| call.into_transaction(target))
        .collect();
    let mut verify_chain = chain.clone();
    let exec = verify_chain.exec(&transactions).unwrap();
    assert!(exec.has_failure(false), "shrunk item must still fail");
}

#[test]
fn no_fail_harness_produces_no_assertions() {
    let (_, failed_assertions) = run_fuzzer("src/NoFail.sol:NoFail", 4, 4, 20, 42);
    assert!(
        failed_assertions.is_empty(),
        "no-fail harness must not produce failed assertions"
    );
}

#[test]
fn single_fail_produces_one_assertion() {
    let (shutdown, failed_assertions) = run_fuzzer("src/SingleFail.sol:SingleFail", 1, 1, 10, 42);

    assert_eq!(
        failed_assertions.len(),
        1,
        "single bug must produce one assertion"
    );
    assert!(
        failed_assertions[0].failure_index.is_some(),
        "assertion must record failure index"
    );
    assert!(
        failed_assertions[0].failure_pc.is_some(),
        "must record assertion PC"
    );
    assert!(
        shutdown.load(Ordering::Relaxed),
        "campaign must stop after the cap"
    );
}

#[test]
fn duplicate_fail_deduplicates_to_one_assertion() {
    let (_, failed_assertions) = run_fuzzer("src/DuplicateFail.sol:DuplicateFail", 8, 1, 100, 42);

    assert_eq!(
        failed_assertions.len(),
        1,
        "argument variations must deduplicate to one assertion"
    );
    assert!(
        failed_assertions[0].failure_pc.is_some(),
        "must record assertion PC"
    );
    assert_eq!(failed_assertions[0].item.calls.len(), 1);
}

#[test]
fn multi_fail_collects_two_distinct_assertions() {
    let (_, failed_assertions) = run_fuzzer("src/MultiFail.sol:MultiFail", 2, 1, 20, 42);

    assert_eq!(
        failed_assertions.len(),
        2,
        "two independent bugs must produce two failed assertions"
    );
    let keys: HashSet<String> = failed_assertions
        .iter()
        .map(|assertion| assertion.dedup_key())
        .collect();
    assert_eq!(keys.len(), 2, "dedupe keys must be distinct");
    let pcs: HashSet<(B256, usize)> = failed_assertions
        .iter()
        .filter_map(|assertion| assertion.failure_pc)
        .collect();
    assert_eq!(pcs.len(), 2, "each bug must have its own assertion PC");
}

#[test]
fn max_failures_cap_stops_after_one_assertion() {
    let (shutdown, failed_assertions) = run_fuzzer("src/MultiFail.sol:MultiFail", 1, 1, 20, 42);

    assert_eq!(
        failed_assertions.len(),
        1,
        "cap must stop collection at one assertion"
    );
    assert!(
        shutdown.load(Ordering::Relaxed),
        "campaign must stop after the cap"
    );
}

#[test]
fn same_function_different_assertions_are_distinct() {
    let (_, failed_assertions) =
        run_fuzzer("src/MultiAssertFail.sol:MultiAssertFail", 2, 1, 50, 42);

    assert_eq!(
        failed_assertions.len(),
        2,
        "two assertions in one function must produce two failed assertions"
    );
    let pcs: HashSet<(B256, usize)> = failed_assertions
        .iter()
        .filter_map(|assertion| assertion.failure_pc)
        .collect();
    assert_eq!(
        pcs.len(),
        2,
        "two assertions in one function must have distinct PCs"
    );
}

#[test]
fn multi_fail_each_assertion_shrinks_to_minimal_sequence() {
    let contract = load_contract("src/MultiFail.sol:MultiFail");
    let (chain, target) = deploy_contract(&contract);
    let (_, failed_assertions) = run_fuzzer("src/MultiFail.sol:MultiFail", 2, 1, 20, 42);
    assert_eq!(failed_assertions.len(), 2);

    let all_functions: Vec<alloy_json_abi::Function> = contract
        .handler_functions
        .iter()
        .chain(contract.invariant_functions.iter())
        .cloned()
        .collect();
    let signatures = signatures(&contract);

    for assertion in &failed_assertions {
        let shared_failed_item = SharedFailedCorpusItem::new(
            assertion.item.clone(),
            CorpusConfig::new(PathBuf::new()).handler_functions(all_functions.clone()),
        );
        let config = ShrinkerConfig::new()
            .chain(chain.clone())
            .target_address(target)
            .shared_failed_item(shared_failed_item.clone())
            .shutdown_signal(Arc::new(AtomicBool::new(false)))
            .max_runs(200)
            .seed(42)
            .shared_metrics(SharedMetrics::new(signatures.clone()))
            .fail_on_revert(false);

        Shrinker::new(config).run().unwrap();
        let shrunk = shared_failed_item.item();
        assert_eq!(
            shrunk.calls.len(),
            1,
            "each assertion must shrink to one call"
        );
        assert_still_fails(&chain, target, &shrunk);
    }
}
