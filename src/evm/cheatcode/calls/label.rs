//! `label` / `getLabel` cheatcodes.

use alloy_dyn_abi::DynSolValue;
use revm::primitives::Address;

use crate::evm::cheatcode::{outcome, state::ExecutionState};

pub fn label(
    state: &mut ExecutionState,
    addr: Address,
    name: &str,
) -> Option<revm::interpreter::CallOutcome> {
    state.labels.insert(addr, name.into());
    Some(outcome::success())
}

pub fn get_label(
    state: &mut ExecutionState,
    addr: Address,
) -> Option<revm::interpreter::CallOutcome> {
    let name = state.labels.get(&addr).cloned().unwrap_or_default();
    let encoded = DynSolValue::String(name).abi_encode();
    Some(outcome::success_bytes(encoded))
}

#[cfg(test)]
mod tests {
    use alloy_dyn_abi::{DynSolType, DynSolValue};
    use alloy_primitives::{Address, U256, address};
    use alloy_sol_types::SolCall;
    use revm::primitives::Bytes;

    use crate::evm::chain::{Chain, DEFAULT_DEPLOYER, DeployInput, SetupInput};
    use crate::evm::cheatcode;
    use crate::evm::cheatcode::calls::label;
    use crate::evm::cheatcode::state::ExecutionState;
    use crate::evm::result::TransactionResult;

    use crate::foundry;
    use crate::target::Contract;

    alloy_sol_types::sol! {
        interface LabelTarget {
            function getLabelFor(address addr) external view returns (string memory label);
            function getStoredLabel() external view returns (string memory label);
            function callLabelSameValueTwice() external returns (string memory first, string memory second);
            function callLabelSequence() external returns (string memory first, string memory second, string memory third);
            function callLabelAndWarp() external returns (string memory label, uint256 timestamp);
            function getUnlabeled(address addr) external view returns (string memory label);
            function setup() external;
            function actionLabel() external;
            function invariant_label() external view;
        }
    }

    const LABEL_ADDR: Address = address!("0xDeaDbeefdEAdbeefdEadbEEFdeadbeEFdEaDbeeF");
    const EXPECTED_LABEL: &str = "DeadBeef";

    fn load_fixture(id: &str) -> Contract {
        let project = foundry::Project::new("fixtures/target-contract-with-cheatcodes");
        let artifacts = project.load_artifacts().unwrap();
        let artifact_id = foundry::ArtifactId::try_from(id).unwrap();
        let artifact = artifacts.get(&artifact_id).unwrap();
        Contract::try_from(artifact).unwrap()
    }

    /// Deploy the fixture and run its `setup` function.
    fn deploy_and_setup() -> (Chain, Address) {
        let contract = load_fixture("src/LabelTarget.sol:LabelTarget");
        let mut chain = Chain::empty();
        let deployment = chain.deploy(DeployInput::new(contract.initcode)).unwrap();
        assert!(deployment.result.success, "deployment must succeed");
        let target = deployment.address.unwrap();

        let setup_opts = SetupInput::new(target);
        let setup = chain.setup(setup_opts).unwrap();
        assert!(setup.result.success, "setup must succeed");

        (chain, target)
    }

    /// Execute a CALL with the cheatcode inspector enabled so that `vm.*`
    /// functions invoked by the target contract are intercepted.
    fn call_with_cheatcode_inspector(
        chain: &mut Chain,
        caller: Address,
        target: Address,
        data: Bytes,
    ) -> TransactionResult {
        let inspector = cheatcode::Inspector::default();
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

    /// Execute a CALL with a given cheatcode inspector and return both the
    /// result and the inspector so state (e.g. labels) can be reused across
    /// transactions.
    fn inspect_with_cheatcode_inspector(
        chain: &mut Chain,
        caller: Address,
        target: Address,
        data: Bytes,
        inspector: cheatcode::Inspector,
    ) -> (TransactionResult, cheatcode::Inspector) {
        let tx = revm::context::TxEnv {
            caller,
            kind: revm::primitives::TxKind::Call(target),
            data,
            gas_limit: u64::MAX,
            value: U256::ZERO,
            ..Default::default()
        };
        chain.inspect(tx, inspector).unwrap()
    }

    /// Call a view/pure function that returns a single `string` and decode it.
    macro_rules! call_string_getter {
        ($chain:expr, $target:expr, $call:ty) => {{
            let calldata = <$call>::new(()).abi_encode();
            let result = $chain
                .call(DEFAULT_DEPLOYER, $target, U256::ZERO, Bytes::from(calldata))
                .unwrap();
            assert!(result.success, "{} must succeed", <$call>::SIGNATURE);
            let output = result.output.expect("getter must return output");
            <$call>::abi_decode_returns(&output).unwrap()
        }};
        ($chain:expr, $target:expr, $call:ty, $args:tt) => {{
            let calldata = <$call>::new($args).abi_encode();
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

    /// vm.label must store the name in execution state.
    #[test]
    fn label_stores_name_in_execution_state() {
        let mut state = ExecutionState::default();
        let outcome = label::label(&mut state, LABEL_ADDR, EXPECTED_LABEL);
        assert!(outcome.is_some(), "must return an outcome");
        let outcome = outcome.unwrap();
        assert!(outcome.result.is_ok(), "vm.label must succeed");
        assert_eq!(
            state.labels.get(&LABEL_ADDR),
            Some(&EXPECTED_LABEL.into()),
            "label must be stored in execution state"
        );
    }

    /// vm.getLabel must return the stored name for a labeled address.
    #[test]
    fn get_label_returns_stored_name() {
        let mut state = ExecutionState::default();
        state.labels.insert(LABEL_ADDR, EXPECTED_LABEL.into());
        let outcome = label::get_label(&mut state, LABEL_ADDR);
        assert!(outcome.is_some(), "must return an outcome");
        let outcome = outcome.unwrap();
        assert!(outcome.result.is_ok(), "vm.getLabel must succeed");
        let string_type = DynSolType::String;
        let decoded = string_type
            .abi_decode_params(&outcome.result.output)
            .unwrap();
        assert_eq!(
            decoded,
            DynSolValue::String(EXPECTED_LABEL.into()),
            "vm.getLabel must return the stored name"
        );
    }

    /// vm.getLabel on an unlabeled address must return an empty string.
    #[test]
    fn get_label_unknown_address_returns_empty() {
        let mut state = ExecutionState::default();
        let outcome = label::get_label(&mut state, LABEL_ADDR);
        assert!(outcome.is_some(), "must return an outcome");
        let outcome = outcome.unwrap();
        assert!(outcome.result.is_ok(), "vm.getLabel must succeed");
        let string_type = DynSolType::String;
        let decoded = string_type
            .abi_decode_params(&outcome.result.output)
            .unwrap();
        assert_eq!(
            decoded,
            DynSolValue::String(String::new()),
            "vm.getLabel on unknown address must return empty string"
        );
    }

    /// vm.label must overwrite an existing label for the same address.
    #[test]
    fn label_overwrites_existing_name() {
        let mut state = ExecutionState::default();
        label::label(&mut state, LABEL_ADDR, "Old");
        label::label(&mut state, LABEL_ADDR, EXPECTED_LABEL);
        let outcome = label::get_label(&mut state, LABEL_ADDR);
        let string_type = DynSolType::String;
        let decoded = string_type
            .abi_decode_params(&outcome.unwrap().result.output)
            .unwrap();
        assert_eq!(
            decoded,
            DynSolValue::String(EXPECTED_LABEL.into()),
            "vm.label must overwrite the previous label"
        );
    }

    /// vm.label with an empty string must store the empty label.
    #[test]
    fn label_empty_string_is_allowed() {
        let mut state = ExecutionState::default();
        let outcome = label::label(&mut state, LABEL_ADDR, "");
        assert!(outcome.is_some(), "must return an outcome");
        assert!(outcome.unwrap().result.is_ok(), "vm.label('') must succeed");
        let outcome = label::get_label(&mut state, LABEL_ADDR);
        let string_type = DynSolType::String;
        let decoded = string_type
            .abi_decode_params(&outcome.unwrap().result.output)
            .unwrap();
        assert_eq!(
            decoded,
            DynSolValue::String(String::new()),
            "vm.getLabel must return empty string after vm.label('')"
        );
    }

    // -----------------------------------------------------------------------
    // Integration tests
    // -----------------------------------------------------------------------

    /// The label stored in contract storage during setup must be readable via
    /// the contract getter in a later transaction, proving persistence.
    #[test]
    fn label_persists_in_storage() {
        let (mut chain, target) = deploy_and_setup();
        let decoded = call_string_getter!(&mut chain, target, LabelTarget::getStoredLabelCall);
        assert_eq!(
            decoded, EXPECTED_LABEL,
            "stored label must persist across transactions"
        );
    }

    /// vm.label followed by vm.getLabel twice in one tx must yield the same
    /// string, proving determinism.
    #[test]
    fn label_same_value_twice_in_sequence_is_deterministic() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = LabelTarget::callLabelSameValueTwiceCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callLabelSameValueTwice() must succeed");
        let output = result.output.expect("must return output");
        let ret = LabelTarget::callLabelSameValueTwiceCall::abi_decode_returns(&output).unwrap();
        assert_eq!(
            ret.first, ret.second,
            "same label must give identical getLabel readings"
        );
        assert_eq!(ret.first, "Self");
    }

    /// vm.label with different values interleaved must produce distinct
    /// getLabel readings, proving the cheatcode is stateful.
    #[test]
    fn label_sequence_returns_consistent_values() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = LabelTarget::callLabelSequenceCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callLabelSequence() must succeed");
        let output = result.output.expect("must return output");
        let ret = LabelTarget::callLabelSequenceCall::abi_decode_returns(&output).unwrap();
        assert_eq!(ret.first, "First", "first vm.label must read First");
        assert_eq!(ret.second, "Second", "second vm.label must read Second");
        assert_eq!(ret.third, "First", "third vm.label must read First again");
    }

    /// vm.label must work correctly when combined with vm.warp in the same tx.
    #[test]
    fn label_interacts_with_warp() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = LabelTarget::callLabelAndWarpCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callLabelAndWarp() must succeed");
        let output = result.output.expect("must return output");
        let ret = LabelTarget::callLabelAndWarpCall::abi_decode_returns(&output).unwrap();
        assert_eq!(ret.label, "Labeled", "label must match the expected value");
        assert_eq!(
            ret.timestamp,
            U256::from(1_234_567_890u64),
            "timestamp must match warped value"
        );
    }

    /// vm.getLabel on an unlabeled address must return an empty string when
    /// called through the contract path.
    #[test]
    fn get_label_unlabeled_returns_empty_string() {
        let (mut chain, target) = deploy_and_setup();
        let unknown = address!("0x00000000000000000000000000000000000000ab");
        let calldata = LabelTarget::getUnlabeledCall::new((unknown,)).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "getUnlabeled must succeed");
        let output = result.output.expect("must return output");
        let decoded = LabelTarget::getUnlabeledCall::abi_decode_returns(&output).unwrap();
        assert_eq!(
            decoded, "",
            "vm.getLabel on unlabeled address must return empty string"
        );
    }

    /// vm.label set in one transaction and vm.getLabel read in the next
    /// transaction must return the same label when the inspector state is
    /// shared, proving cross-transaction persistence of label state.
    #[test]
    fn label_persists_across_transactions_with_shared_inspector() {
        let (mut chain, target) = deploy_and_setup();
        let inspector = cheatcode::Inspector::default();

        // Tx 1: label an address.
        let calldata = LabelTarget::actionLabelCall::new(()).abi_encode();
        let (result, inspector) = inspect_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
            inspector,
        );
        assert!(result.success, "actionLabel must succeed");

        // Tx 2: getLabel for the same address without re-labeling.
        let calldata = LabelTarget::getLabelForCall::new((LABEL_ADDR,)).abi_encode();
        let (result, _inspector) = inspect_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
            inspector,
        );
        assert!(result.success, "getLabelFor must succeed");
        let output = result.output.expect("must return output");
        let ret = LabelTarget::getLabelForCall::abi_decode_returns(&output).unwrap();
        assert_eq!(
            ret, EXPECTED_LABEL,
            "vm.getLabel must return the label set in the previous transaction"
        );
    }

    /// Invariant must pass immediately after setup (fuzzing baseline).
    #[test]
    fn invariant_passes_after_setup() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = LabelTarget::invariant_labelCall::new(()).abi_encode();
        let result = chain
            .call(DEFAULT_DEPLOYER, target, U256::ZERO, Bytes::from(calldata))
            .unwrap();
        assert!(result.success, "invariant must pass after setup");
    }

    /// A fuzz-like sequence of actions followed by invariants must all succeed.
    /// This proves vm.label stays deterministic across multiple transactions
    /// and that invariants correctly observe the mutated state.
    #[test]
    fn action_sequence_and_invariants() {
        let (mut chain, target) = deploy_and_setup();
        let mut inspector = cheatcode::Inspector::default();

        // Action 1: re-label the expected address and store it.
        let calldata = LabelTarget::actionLabelCall::new(()).abi_encode();
        let (result, insp) = inspect_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
            inspector,
        );
        assert!(result.success, "actionLabel must succeed");
        inspector = insp;

        // Invariant must still pass after the action.
        let calldata = LabelTarget::invariant_labelCall::new(()).abi_encode();
        let result = chain
            .call(DEFAULT_DEPLOYER, target, U256::ZERO, Bytes::from(calldata))
            .unwrap();
        assert!(result.success, "invariant must pass after actionLabel");

        // Action 2: temporarily relabel and then restore via sequence.
        let calldata = LabelTarget::callLabelSequenceCall::new(()).abi_encode();
        let (result, insp) = inspect_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
            inspector,
        );
        assert!(result.success, "callLabelSequence must succeed");
        inspector = insp;

        // Action 3: restore the expected label.
        let calldata = LabelTarget::actionLabelCall::new(()).abi_encode();
        let (result, _inspector) = inspect_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
            inspector,
        );
        assert!(result.success, "actionLabel must succeed on restore");

        // Invariant must pass after restoring state.
        let calldata = LabelTarget::invariant_labelCall::new(()).abi_encode();
        let result = chain
            .call(DEFAULT_DEPLOYER, target, U256::ZERO, Bytes::from(calldata))
            .unwrap();
        assert!(result.success, "invariant must pass after action sequence");
    }

    /// vm.label(expected) followed by vm.getLabel must return the same label
    /// when called in a separate transaction after the initial setup, proving
    /// cross-transaction determinism via contract storage.
    #[test]
    fn label_deterministic_across_transaction_sequence() {
        let (mut chain, target) = deploy_and_setup();

        let calldata = LabelTarget::actionLabelCall::new(()).abi_encode();

        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata.clone()),
        );
        assert!(result.success, "actionLabel must succeed");
        let stored = call_string_getter!(&mut chain, target, LabelTarget::getStoredLabelCall);
        assert_eq!(
            stored, EXPECTED_LABEL,
            "stored label must match after first action"
        );

        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "actionLabel must succeed on second call");
        let stored = call_string_getter!(&mut chain, target, LabelTarget::getStoredLabelCall);
        assert_eq!(
            stored, EXPECTED_LABEL,
            "stored label must still match after second action"
        );
    }
}
