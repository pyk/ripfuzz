//! `getCode` cheatcode - read compiled initcode by contract artifact id.

use crate::evm::cheatcode::{outcome, state::ExecutionState};

pub fn handle(name: &str, state: &mut ExecutionState) -> Option<revm::interpreter::CallOutcome> {
    let initcode = state.compiled_contracts.get(name)?;
    if initcode.is_empty() {
        return Some(outcome::revert(&format!(
            "getCode: bytecode is empty: {name}"
        )));
    }
    let encoded = alloy_dyn_abi::DynSolValue::Bytes(initcode.to_vec()).abi_encode();
    Some(outcome::success_bytes(encoded))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use alloy_primitives::{Address, U256};
    use alloy_sol_types::SolCall;
    use revm::primitives::Bytes;

    use crate::evm::Contract;
    use crate::evm::chain::{Chain, ChainConfig, DeployInput, SetupInput, Transaction};
    use crate::evm::cheatcode::calls::get_code;
    use crate::evm::cheatcode::state::ExecutionState;
    use crate::foundry;

    alloy_sol_types::sol! {
        interface GetCodeHandler {
            function setup() external;
            function actionGetCode() external;
            function actionMutateGetCode() external;
            function actionGetCodeSequence()
                external
                returns (uint256 first, uint256 second, uint256 third);
            function getDeployedValue() external view returns (uint256);
            function invariant_getCode() external view;
        }
    }

    const EXPECTED_VALUE: U256 = U256::from_limbs([42, 0, 0, 0]);
    const COUNTER_ARTIFACT_ID: &str = "src/Counter.sol:Counter";

    fn load_fixture(id: &str) -> Contract {
        let project = foundry::Project::new("fixtures/handler-contract-with-cheatcodes");
        let artifacts = project.load_artifacts().unwrap();
        let artifact_id = foundry::ArtifactId::try_from(id).unwrap();
        Contract::try_get(&artifacts, &artifact_id).unwrap()
    }

    /// Load compiled initcode from the fixture project and return a map keyed
    /// by full artifact id (`src/Counter.sol:Counter`).
    fn load_compiled_contracts() -> HashMap<String, Bytes> {
        let mut map = HashMap::new();
        let project = foundry::Project::new("fixtures/handler-contract-with-cheatcodes");
        let artifacts = project.load_artifacts().unwrap();

        for (id, artifact) in &artifacts {
            let initcode: Bytes = match artifact {
                foundry::Artifact::Contract(c) => c.bytecode.object.parse().unwrap_or_default(),
                foundry::Artifact::Library(c) => c.bytecode.object.parse().unwrap_or_default(),
                _ => continue,
            };
            if initcode.is_empty() {
                continue;
            }
            map.insert(id.into(), initcode);
        }
        map
    }

    fn deploy_and_setup() -> (Chain, Address) {
        let contract = load_fixture("src/GetCodeHandler.sol:GetCodeHandler");
        let config = ChainConfig::new("fixtures/handler-contract-with-cheatcodes")
            .with_compiled_contracts(load_compiled_contracts());
        let mut chain = Chain::new(config).unwrap();
        let deployment = chain.deploy(DeployInput::new(&contract.initcode)).unwrap();
        assert!(deployment.result.success, "deployment must succeed");
        let target = deployment.address.unwrap();

        let setup = chain.setup(SetupInput::new(target)).unwrap();
        assert!(setup.result.success, "setup must succeed");

        (chain, target)
    }

    /// vm.getCode with a valid full artifact id must return non-empty bytecode.
    #[test]
    fn get_code_returns_bytecode_for_valid_artifact_id() {
        let mut state = ExecutionState::default();
        let initcode = Bytes::from(vec![
            0x60, 0x2a, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xf3,
        ]);
        state
            .compiled_contracts
            .insert(COUNTER_ARTIFACT_ID.into(), initcode.clone());

        let outcome = get_code::handle(COUNTER_ARTIFACT_ID, &mut state);
        assert!(outcome.is_some(), "must return an outcome");
        let outcome = outcome.unwrap();
        assert!(outcome.result.is_ok(), "vm.getCode must succeed");
    }

    /// vm.getCode must return `None` when the requested artifact is not known.
    #[test]
    fn get_code_unknown_artifact_returns_none() {
        let mut state = ExecutionState::default();
        let outcome = get_code::handle("src/Unknown.sol:Unknown", &mut state);
        assert!(outcome.is_none(), "unknown artifact must return None");
    }

    /// vm.getCode must revert with an explicit message when the bytecode is empty.
    #[test]
    fn get_code_empty_bytecode_reverts() {
        let mut state = ExecutionState::default();
        state
            .compiled_contracts
            .insert("src/Empty.sol:Empty".into(), Bytes::new());
        let outcome = get_code::handle("src/Empty.sol:Empty", &mut state);
        assert!(outcome.is_some(), "must return an outcome");
        let outcome = outcome.unwrap();
        assert!(
            !outcome.result.is_ok(),
            "vm.getCode must revert when bytecode is empty"
        );
    }

    /// `vm.getCode` used during setup must persist the deployed contract so
    /// that a later view call can read the expected value.
    #[test]
    fn get_code_set_in_setup_matches_expected() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                GetCodeHandler::getDeployedValueCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                GetCodeHandler::invariant_getCodeCall::new(()).abi_encode(),
            )),
        ];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(
            execution.results[0].success,
            "getDeployedValue must return the deployed value"
        );
        let stored: U256 = GetCodeHandler::getDeployedValueCall::abi_decode_returns(
            &execution.results[0].output.clone().unwrap(),
        )
        .unwrap();
        assert_eq!(stored, EXPECTED_VALUE, "deployed value must match expected");
        assert!(
            execution.results[1].success,
            "invariant must pass after setup"
        );
    }

    /// Re-deploying the same initcode via `vm.getCode` in a later transaction
    /// must restore the canonical value.
    #[test]
    fn restore_get_code_in_action_preserves_value() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                GetCodeHandler::actionGetCodeCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                GetCodeHandler::invariant_getCodeCall::new(()).abi_encode(),
            )),
        ];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(execution.results[0].success, "actionGetCode must succeed");
        assert!(
            execution.results[1].success,
            "invariant must pass after restoring value"
        );
    }

    /// A single transaction can interleave multiple `vm.getCode` calls with
    /// different artifacts and end on the expected value. This proves the
    /// cheatcode is deterministic and safe to call repeatedly inside one tx.
    #[test]
    fn batch_sequence_in_single_transaction() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                GetCodeHandler::actionGetCodeSequenceCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                GetCodeHandler::invariant_getCodeCall::new(()).abi_encode(),
            )),
        ];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(
            execution.results[0].success,
            "actionGetCodeSequence must succeed"
        );
        let output = execution.results[0].output.clone().unwrap();
        let ret = GetCodeHandler::actionGetCodeSequenceCall::abi_decode_returns(&output).unwrap();
        assert_eq!(ret.first, EXPECTED_VALUE, "first getCode must read 42");
        assert_eq!(ret.second, U256::from(100), "second getCode must read 100");
        assert_eq!(ret.third, EXPECTED_VALUE, "third getCode must read 42");
        assert!(
            execution.results[1].success,
            "invariant must pass after sequence"
        );
    }

    /// Mutating the deployed contract via a different `vm.getCode` artifact
    /// and then restoring it in a sequence must leave the invariant intact.
    #[test]
    fn mutate_and_restore_preserves_invariant() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                GetCodeHandler::actionMutateGetCodeCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                GetCodeHandler::actionGetCodeCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                GetCodeHandler::invariant_getCodeCall::new(()).abi_encode(),
            )),
        ];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 3);
        assert!(
            execution.results[0].success,
            "actionMutateGetCode must succeed"
        );
        assert!(execution.results[1].success, "actionGetCode must succeed");
        assert!(
            execution.results[2].success,
            "invariant must pass after mutate and restore"
        );
    }

    /// A cloned chain snapshot must produce the same deployed value when
    /// actions are executed on the clone. This is critical for parallel
    /// fuzzing where each worker starts from a cloned state.
    #[test]
    fn cloned_chain_preserves_get_code() {
        let (chain, target) = deploy_and_setup();
        let mut cloned = chain.clone();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                GetCodeHandler::actionGetCodeCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                GetCodeHandler::invariant_getCodeCall::new(()).abi_encode(),
            )),
        ];

        let execution = cloned.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(
            execution.results[0].success,
            "actionGetCode must succeed on cloned chain"
        );
        assert!(
            execution.results[1].success,
            "invariant must pass on cloned chain"
        );
    }

    /// A realistic fuzzing sequence: multiple actions mutate contract state
    /// by re-deploying via `vm.getCode`, and a final invariant verifies that
    /// the canonical value is still intact.
    #[test]
    fn deterministic_across_transaction_sequence() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                GetCodeHandler::actionGetCodeCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                GetCodeHandler::actionMutateGetCodeCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                GetCodeHandler::actionGetCodeCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                GetCodeHandler::actionGetCodeSequenceCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                GetCodeHandler::invariant_getCodeCall::new(()).abi_encode(),
            )),
        ];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 5);
        assert!(
            execution.results.iter().all(|r| r.success),
            "all sequence steps must succeed"
        );
    }
}
