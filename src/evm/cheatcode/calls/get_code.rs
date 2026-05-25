//! `getCode` cheatcode - read compiled initcode by contract artifact id.

use crate::evm::cheatcode::{outcome, state::ExecutionState};

pub fn handle(name: &str, state: &mut ExecutionState) -> Option<revm::interpreter::CallOutcome> {
    let initcode = state.compiled_contracts.get(name).or_else(|| {
        name.rsplit(':')
            .next()
            .and_then(|short| state.compiled_contracts.get(short))
    })?;
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
    use alloy_primitives::{Address, U256};
    use alloy_sol_types::SolCall;
    use revm::primitives::Bytes;

    use crate::evm::chain::{Chain, DEFAULT_DEPLOYER, DeployInput};
    use crate::evm::cheatcode;
    use crate::evm::cheatcode::calls::get_code;
    use crate::evm::cheatcode::state::ExecutionState;
    use crate::evm::result::TransactionResult;

    use crate::foundry;
    use crate::target::Contract;

    alloy_sol_types::sol! {
        interface GetCodeTarget {
            function getDeployedValue() external view returns (uint256);
            function getStoredValue() external view returns (uint256);
            function callGetCodeSameValueTwice() external returns (uint256 first, uint256 second);
            function callGetCodeSequence() external returns (uint256 first, uint256 second, uint256 third);
            function callGetCodeAndWarp() external returns (uint256 value, uint256 timestamp);
            function setup() external;
            function actionGetCode() external;
            function invariant_get_code() external view;
        }
    }

    const EXPECTED_VALUE: U256 = U256::from_limbs([42, 0, 0, 0]);
    const COUNTER_ARTIFACT_ID: &str = "src/Counter.sol:Counter";

    fn load_fixture(id: &str) -> Contract {
        let project = foundry::Project::new("fixtures/target-contract-with-cheatcodes");
        let artifacts = project.load_artifacts().unwrap();
        let artifact_id = foundry::ArtifactId::try_from(id).unwrap();
        let artifact = artifacts.get(&artifact_id).unwrap();
        Contract::try_from(artifact).unwrap()
    }

    /// Build a cheatcode inspector whose `compiled_contracts` map is seeded
    /// from the fixture project so `vm.getCode` can resolve artifact ids.
    fn get_code_enabled_inspector() -> cheatcode::Inspector {
        let mut inspector = cheatcode::Inspector::default();
        let project = foundry::Project::new("fixtures/target-contract-with-cheatcodes");
        let artifacts = project.load_artifacts().unwrap();

        for (id, artifact) in &artifacts {
            let initcode = match artifact {
                crate::foundry::Artifact::Contract(c) => {
                    crate::contract::artifact::parse_hex(&c.bytecode.object).unwrap_or_default()
                }
                crate::foundry::Artifact::Library(c) => {
                    crate::contract::artifact::parse_hex(&c.bytecode.object).unwrap_or_default()
                }
                _ => continue,
            };
            if initcode.is_empty() {
                continue;
            }
            // Support both full artifact id (`src/Counter.sol:Counter`) and short name (`Counter`).
            inspector
                .state
                .compiled_contracts
                .insert(id.into(), initcode.clone());
            inspector
                .state
                .compiled_contracts
                .insert(id.name.clone(), initcode);
        }
        inspector
    }

    /// Deploy the fixture and run its `setup` function with a getCode-enabled inspector.
    fn deploy_and_setup() -> (Chain, Address) {
        let contract = load_fixture("src/GetCodeTarget.sol:GetCodeTarget");
        let mut chain = Chain::empty();
        let deployment = chain.deploy(DeployInput::new(contract.initcode)).unwrap();
        assert!(deployment.result.success, "deployment must succeed");
        let target = deployment.address.unwrap();

        let setup_data = Bytes::from(GetCodeTarget::setupCall::new(()).abi_encode());
        let tx = revm::context::TxEnv {
            caller: DEFAULT_DEPLOYER,
            kind: revm::primitives::TxKind::Call(target),
            data: setup_data,
            gas_limit: u64::MAX,
            value: U256::ZERO,
            ..Default::default()
        };
        let inspector = get_code_enabled_inspector();
        let (result, _) = chain.inspect(tx, inspector).unwrap();
        assert!(result.success, "setup must succeed");

        (chain, target)
    }

    /// Execute a CALL with the cheatcode inspector enabled so that `vm.*`
    /// functions invoked by the target contract are intercepted.
    fn call_with_get_code_inspector(
        chain: &mut Chain,
        caller: Address,
        target: Address,
        data: Bytes,
    ) -> TransactionResult {
        let inspector = get_code_enabled_inspector();
        let tx = revm::context::TxEnv {
            caller,
            kind: revm::primitives::TxKind::Call(target),
            data,
            gas_limit: u64::MAX,
            value: U256::ZERO,
            ..Default::default()
        };
        let (result, _) = chain.inspect(tx, inspector).unwrap();
        result
    }

    /// Call a view/pure function that returns a single `uint256` and decode it.
    macro_rules! call_uint256_getter {
        ($chain:expr, $target:expr, $call:ty) => {{
            let calldata = <$call>::new(()).abi_encode();
            let result = $chain
                .call(DEFAULT_DEPLOYER, $target, U256::ZERO, Bytes::from(calldata))
                .unwrap();
            assert!(result.success, "{} must succeed", <$call>::SIGNATURE);
            let output = result.output.expect("getter must return output");
            <$call>::abi_decode_returns(&output).unwrap()
        }};
    }

    // -----------------------------------------------------------------------
    // Handler-level (direct Rust unit tests)
    // -----------------------------------------------------------------------

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

    /// vm.getCode with a short contract name must also resolve when the long
    /// form is not present.
    #[test]
    fn get_code_returns_bytecode_for_short_name() {
        let mut state = ExecutionState::default();
        let initcode = Bytes::from(vec![
            0x60, 0x2a, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xf3,
        ]);
        state.compiled_contracts.insert("Counter".into(), initcode);

        let outcome = get_code::handle("Counter", &mut state);
        assert!(outcome.is_some(), "must return an outcome");
        let outcome = outcome.unwrap();
        assert!(outcome.result.is_ok(), "vm.getCode must succeed");
    }

    /// vm.getCode with a full artifact id must fall back to the short name
    /// when the exact key is missing.
    #[test]
    fn get_code_fallback_to_short_name() {
        let mut state = ExecutionState::default();
        let initcode = Bytes::from(vec![
            0x60, 0x2a, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xf3,
        ]);
        state.compiled_contracts.insert("Counter".into(), initcode);

        let outcome = get_code::handle(COUNTER_ARTIFACT_ID, &mut state);
        assert!(outcome.is_some(), "must return an outcome");
        let outcome = outcome.unwrap();
        assert!(
            outcome.result.is_ok(),
            "vm.getCode with artifact id must fallback to short name"
        );
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
            .insert("Empty".into(), Bytes::new());
        let outcome = get_code::handle("Empty", &mut state);
        assert!(outcome.is_some(), "must return an outcome");
        let outcome = outcome.unwrap();
        assert!(
            !outcome.result.is_ok(),
            "vm.getCode must revert when bytecode is empty"
        );
    }

    // -----------------------------------------------------------------------
    // Integration tests
    // -----------------------------------------------------------------------

    /// The value deployed via vm.getCode during setup must be readable via the
    /// contract getter in a later transaction, proving persistence.
    #[test]
    fn get_code_persists_in_storage() {
        let (mut chain, target) = deploy_and_setup();
        let decoded: U256 =
            call_uint256_getter!(&mut chain, target, GetCodeTarget::getDeployedValueCall);
        assert_eq!(
            decoded, EXPECTED_VALUE,
            "deployed value must persist across transactions"
        );
    }

    /// vm.getCode with the same artifact twice in one tx must yield identical
    /// deployed values, proving determinism.
    #[test]
    fn get_code_same_value_twice_in_sequence_is_deterministic() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = GetCodeTarget::callGetCodeSameValueTwiceCall::new(()).abi_encode();
        let result = call_with_get_code_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callGetCodeSameValueTwice() must succeed");
        let output = result.output.expect("must return output");
        let ret =
            GetCodeTarget::callGetCodeSameValueTwiceCall::abi_decode_returns(&output).unwrap();
        assert_eq!(
            ret.first, ret.second,
            "same getCode artifact must give identical deployed values"
        );
        assert_eq!(ret.first, EXPECTED_VALUE);
    }

    /// vm.getCode with different artifacts interleaved must produce distinct
    /// deployed values, proving the cheatcode responds to different inputs.
    #[test]
    fn get_code_sequence_returns_consistent_values() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = GetCodeTarget::callGetCodeSequenceCall::new(()).abi_encode();
        let result = call_with_get_code_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callGetCodeSequence() must succeed");
        let output = result.output.expect("must return output");
        let ret = GetCodeTarget::callGetCodeSequenceCall::abi_decode_returns(&output).unwrap();
        assert_eq!(
            ret.first, EXPECTED_VALUE,
            "first vm.getCode(Counter) must read 42"
        );
        assert_eq!(
            ret.second,
            U256::from_limbs([100, 0, 0, 0]),
            "second vm.getCode(AltCounter) must read 100"
        );
        assert_eq!(
            ret.third, EXPECTED_VALUE,
            "third vm.getCode(Counter) must read 42 again"
        );
    }

    /// vm.getCode must work correctly when combined with vm.warp in the same tx.
    #[test]
    fn get_code_interacts_with_warp() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = GetCodeTarget::callGetCodeAndWarpCall::new(()).abi_encode();
        let result = call_with_get_code_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callGetCodeAndWarp() must succeed");
        let output = result.output.expect("must return output");
        let ret = GetCodeTarget::callGetCodeAndWarpCall::abi_decode_returns(&output).unwrap();
        assert_eq!(
            ret.value, EXPECTED_VALUE,
            "deployed value must match expected"
        );
        assert_eq!(
            ret.timestamp,
            U256::from(1_234_567_890u64),
            "timestamp must match warped value"
        );
    }

    /// Invariant must pass immediately after setup (fuzzing baseline).
    #[test]
    fn invariant_passes_after_setup() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = GetCodeTarget::invariant_get_codeCall::new(()).abi_encode();
        let result = chain
            .call(DEFAULT_DEPLOYER, target, U256::ZERO, Bytes::from(calldata))
            .unwrap();
        assert!(result.success, "invariant must pass after setup");
    }

    /// A fuzz-like sequence of actions followed by invariants must all succeed.
    /// This proves vm.getCode stays deterministic across multiple transactions
    /// and that invariants correctly observe the mutated state.
    #[test]
    fn action_sequence_and_invariants() {
        let (mut chain, target) = deploy_and_setup();

        // Temporarily mutate deployed code via a sequence that ends on a different value.
        let calldata = GetCodeTarget::callGetCodeSequenceCall::new(()).abi_encode();
        let result = call_with_get_code_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callGetCodeSequence must succeed");

        // Restore the expected code with an action.
        let calldata = GetCodeTarget::actionGetCodeCall::new(()).abi_encode();
        let result = call_with_get_code_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "actionGetCode must succeed");

        // Invariant must pass after the action restored state.
        let calldata = GetCodeTarget::invariant_get_codeCall::new(()).abi_encode();
        let result = chain
            .call(DEFAULT_DEPLOYER, target, U256::ZERO, Bytes::from(calldata))
            .unwrap();
        assert!(result.success, "invariant must pass after action sequence");
    }

    /// vm.getCode(expected) must set the same value when called in a separate
    /// transaction after the initial setup, proving cross-transaction determinism.
    #[test]
    fn get_code_deterministic_across_transaction_sequence() {
        let (mut chain, target) = deploy_and_setup();

        let calldata = GetCodeTarget::actionGetCodeCall::new(()).abi_encode();

        let result = call_with_get_code_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata.clone()),
        );
        assert!(result.success, "actionGetCode must succeed");
        let stored: U256 =
            call_uint256_getter!(&mut chain, target, GetCodeTarget::getStoredValueCall);
        assert_eq!(
            stored, EXPECTED_VALUE,
            "stored value must match after first action"
        );

        let result = call_with_get_code_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "actionGetCode must succeed on second call");
        let stored: U256 =
            call_uint256_getter!(&mut chain, target, GetCodeTarget::getStoredValueCall);
        assert_eq!(
            stored, EXPECTED_VALUE,
            "stored value must still match after second action"
        );
    }
}
