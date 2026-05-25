//! `etch` cheatcode - set contract bytecode at an address.

use revm::{
    bytecode::Bytecode,
    context::BlockEnv,
    context::ContextSetters,
    context_interface::{ContextTr, JournalTr},
    primitives::{Address, Bytes},
};

use crate::evm::cheatcode::outcome;

pub fn handle<CTX: ContextTr + ContextSetters<Block = BlockEnv>>(
    ctx: &mut CTX,
    addr: Address,
    code: Bytes,
) -> Option<revm::interpreter::CallOutcome> {
    if ctx.journal_mut().precompile_addresses().contains(&addr) {
        return Some(outcome::revert("cannot etch precompile address"));
    }
    ctx.journal_mut()
        .load_account(addr)
        .map_err(|_| "account load failed")
        .ok()?;
    let bytecode = Bytecode::new_raw_checked(code)
        .map_err(|e| format!("failed to create bytecode: {e}"))
        .ok()?;
    ctx.journal_mut().set_code(addr, bytecode);
    Some(outcome::success())
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{Address, U256, address};
    use alloy_sol_types::SolCall;
    use revm::MainContext;
    use revm::primitives::Bytes;

    use crate::evm::chain::{Chain, DEFAULT_DEPLOYER, DeployOptions, SetupOptions};
    use crate::evm::cheatcode;
    use crate::evm::cheatcode::calls::etch;
    use crate::evm::result::TransactionResult;

    use crate::foundry;
    use crate::target::Contract;

    alloy_sol_types::sol! {
        interface EtchTarget {
            function getEtchedValue() external view returns (uint256);
            function getStoredValue() external view returns (uint256);
            function callEtchSameValueTwice() external returns (uint256 first, uint256 second);
            function callEtchSequence() external returns (uint256 first, uint256 second, uint256 third);
            function callEtchAndWarp() external returns (uint256 value, uint256 timestamp);
            function setup() external;
            function actionEtch() external;
            function invariant_etch() external view;
        }
    }

    const ETCH_ADDR: Address = address!("0xDeaDbeefdEAdbeefdEadbEEFdeadbeEFdEaDbeeF");
    const EXPECTED_VALUE: U256 = U256::from_limbs([42, 0, 0, 0]);

    fn load_fixture(id: &str) -> Contract {
        let project = foundry::Project::new("fixtures/target-contract-with-cheatcodes");
        let artifacts = project.load_artifacts().unwrap();
        let artifact_id = foundry::ArtifactId::try_from(id).unwrap();
        let artifact = artifacts.get(&artifact_id).unwrap();
        Contract::try_from(artifact).unwrap()
    }

    /// Deploy the fixture and run its `setup` function.
    fn deploy_and_setup() -> (Chain, Address) {
        let contract = load_fixture("src/EtchTarget.sol:EtchTarget");
        let mut chain = Chain::empty();
        let deployment = chain.deploy(DeployOptions::new(contract.initcode)).unwrap();
        assert!(deployment.result.success, "deployment must succeed");
        let target = deployment.address.unwrap();

        let setup_data = Bytes::from(EtchTarget::setupCall::new(()).abi_encode());
        let setup_opts = SetupOptions::new(target, setup_data);
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

    /// vm.etch must return success for a valid address and bytecode.
    #[test]
    fn etch_sets_bytecode_without_reverting() {
        let mut ctx = revm::context::Context::mainnet();
        let code = Bytes::from(vec![
            0x60, 0x2a, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xf3,
        ]);
        let outcome = etch::handle(&mut ctx, ETCH_ADDR, code);
        assert!(outcome.is_some(), "must return an outcome");
    }

    /// The value etched during setup must be readable via the contract getter
    /// in a later transaction, proving cross-transaction persistence.
    #[test]
    fn etch_persists_across_transactions() {
        let (mut chain, target) = deploy_and_setup();
        let decoded: U256 =
            call_uint256_getter!(&mut chain, target, EtchTarget::getEtchedValueCall);
        assert_eq!(
            decoded, EXPECTED_VALUE,
            "etched value must persist across transactions"
        );
    }

    /// vm.etch with the same code twice in one tx must yield the same
    /// reading, proving determinism.
    #[test]
    fn etch_same_value_twice_in_sequence_is_deterministic() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = EtchTarget::callEtchSameValueTwiceCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callEtchSameValueTwice() must succeed");
        let output = result.output.expect("must return output");
        let ret = EtchTarget::callEtchSameValueTwiceCall::abi_decode_returns(&output).unwrap();
        assert_eq!(
            ret.first, ret.second,
            "same etch code must give identical readings"
        );
        assert_eq!(ret.first, EXPECTED_VALUE);
    }

    /// vm.etch with different codes interleaved must produce distinct
    /// readings, proving the cheatcode is stateful.
    #[test]
    fn etch_sequence_returns_consistent_values() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = EtchTarget::callEtchSequenceCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callEtchSequence() must succeed");
        let output = result.output.expect("must return output");
        let ret = EtchTarget::callEtchSequenceCall::abi_decode_returns(&output).unwrap();
        assert_eq!(
            ret.first, EXPECTED_VALUE,
            "first vm.etch(Counter) must read 42"
        );
        assert_eq!(
            ret.second,
            U256::from_limbs([100, 0, 0, 0]),
            "second vm.etch(AltCounter) must read 100"
        );
        assert_eq!(
            ret.third, EXPECTED_VALUE,
            "third vm.etch(Counter) must read 42 again"
        );
    }

    /// vm.etch must work correctly when combined with vm.warp in the same tx.
    #[test]
    fn etch_interacts_with_warp() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = EtchTarget::callEtchAndWarpCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callEtchAndWarp() must succeed");
        let output = result.output.expect("must return output");
        let ret = EtchTarget::callEtchAndWarpCall::abi_decode_returns(&output).unwrap();
        assert_eq!(
            ret.value, EXPECTED_VALUE,
            "etched value must match expected"
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
        let calldata = EtchTarget::invariant_etchCall::new(()).abi_encode();
        let result = chain
            .call(DEFAULT_DEPLOYER, target, U256::ZERO, Bytes::from(calldata))
            .unwrap();
        assert!(result.success, "invariant must pass after setup");
    }

    /// A fuzz-like sequence of actions followed by invariants must all succeed.
    #[test]
    fn action_sequence_and_invariants() {
        let (mut chain, target) = deploy_and_setup();

        // Temporarily mutate etched code via a sequence that ends on a different value.
        let calldata = EtchTarget::callEtchSequenceCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callEtchSequence must succeed");

        // Restore the expected code with an action.
        let calldata = EtchTarget::actionEtchCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "actionEtch must succeed");

        // Invariant must pass after the action restored state.
        let calldata = EtchTarget::invariant_etchCall::new(()).abi_encode();
        let result = chain
            .call(DEFAULT_DEPLOYER, target, U256::ZERO, Bytes::from(calldata))
            .unwrap();
        assert!(result.success, "invariant must pass after action sequence");
    }

    /// vm.etch(expected) must set the same value when called in a separate
    /// transaction after the initial setup, proving cross-transaction determinism.
    #[test]
    fn etch_deterministic_across_transaction_sequence() {
        let (mut chain, target) = deploy_and_setup();

        let calldata = EtchTarget::actionEtchCall::new(()).abi_encode();

        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata.clone()),
        );
        assert!(result.success, "actionEtch must succeed");
        let stored: U256 = call_uint256_getter!(&mut chain, target, EtchTarget::getStoredValueCall);
        assert_eq!(
            stored, EXPECTED_VALUE,
            "stored value must match after first action"
        );

        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "actionEtch must succeed on second call");
        let stored: U256 = call_uint256_getter!(&mut chain, target, EtchTarget::getStoredValueCall);
        assert_eq!(
            stored, EXPECTED_VALUE,
            "stored value must still match after second action"
        );
    }
}
